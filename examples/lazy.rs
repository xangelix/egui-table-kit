//! A lazy table: a billion rows of sixty columns, loaded in blocks as the view scrolls.
//!
//! A loader thread stands in for a file reader: it serves blocks of 1,024 rows after about
//! 30 ms each. The provider asks for the blocks in view from `prepare`, keeps a few dozen of them
//! (least recently used first out), and shows cells as loading until their block arrives. A few
//! cells fail, to show how failures are drawn.

use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use eframe::egui;
use egui_table_kit::{
    error::TableError,
    interaction::SelectionMode,
    layout::Column,
    operations::{
        BorrowedRow, CellStatus, HeaderIter, RowCallback, TableCell, TableProvider, Viewport,
    },
    state::TableState,
    table::TableKit,
};

const ROWS: usize = 1_000_000_000;
const COLUMNS: usize = 60;
const BLOCK_ROWS: usize = 1_024;
/// How many blocks the provider keeps.
const CACHED_BLOCKS: usize = 48;
/// How long the loader takes over a block.
const LOAD_TIME: Duration = Duration::from_millis(30);
/// How many of the latest requests the loader keeps; older ones (scrolled past) are dropped.
const KEPT_REQUESTS: usize = 4;

/// One block's values, row by row.
struct Block(Vec<u64>);

impl Block {
    fn load(block: usize) -> Self {
        let first = block * BLOCK_ROWS;
        let rows = BLOCK_ROWS.min(ROWS - first);
        Self(
            (0..rows * COLUMNS)
                .map(|i| mix((first * COLUMNS + i) as u64))
                .collect(),
        )
    }

    fn value(&self, row: usize, col: usize) -> Option<u64> {
        self.0.get((row % BLOCK_ROWS) * COLUMNS + col).copied()
    }
}

/// SplitMix64: a value per cell, the same each time.
fn mix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

enum Reply {
    Loaded(usize, Block),
    /// Scrolled past before it was served: ask again if it comes back into view.
    Dropped(usize),
}

/// The loader thread: serves the latest request first, one block at a time, and drops requests
/// that fell behind (scrolled past).
fn spawn_loader(ctx: egui::Context) -> (Sender<usize>, Receiver<Reply>) {
    let (requests, incoming) = mpsc::channel::<usize>();
    let (replies, outgoing) = mpsc::channel();
    thread::spawn(move || {
        // Oldest first.
        let mut queue = Vec::new();
        loop {
            if queue.is_empty() {
                match incoming.recv() {
                    Ok(block) => queue.push(block),
                    Err(_) => return,
                }
            }
            queue.extend(incoming.try_iter());
            let stale = queue.len().saturating_sub(KEPT_REQUESTS);
            for block in queue.drain(..stale) {
                if replies.send(Reply::Dropped(block)).is_err() {
                    return;
                }
            }
            let Some(block) = queue.pop() else {
                continue;
            };
            thread::sleep(LOAD_TIME);
            if replies
                .send(Reply::Loaded(block, Block::load(block)))
                .is_err()
            {
                return;
            }
            ctx.request_repaint();
        }
    });
    (requests, outgoing)
}

#[derive(Default)]
struct Cache {
    blocks: HashMap<usize, Block>,
    /// Least recently used first.
    recent: VecDeque<usize>,
    pending: HashSet<usize>,
}

impl Cache {
    fn touch(&mut self, block: usize) {
        if let Some(at) = self.recent.iter().position(|&b| b == block) {
            self.recent.remove(at);
        }
        self.recent.push_back(block);
    }
}

struct LazyRows {
    requests: Sender<usize>,
    replies: Receiver<Reply>,
    cache: RefCell<Cache>,
}

impl LazyRows {
    fn new(ctx: &egui::Context) -> Self {
        let (requests, replies) = spawn_loader(ctx.clone());
        Self {
            requests,
            replies,
            cache: RefCell::default(),
        }
    }

    /// A few cells fail, to show how they are drawn.
    fn fails(row: usize, col: usize) -> bool {
        col == 7 && row % 1_000 == 13
    }

    fn text(row: usize, col: usize, value: u64) -> String {
        match col {
            0 => format!("{row}"),
            _ => match col % 4 {
                1 => format!("{value:016x}"),
                2 => format!("{:.3}", (value % 1_000_000) as f64 / 1_000.0),
                3 => ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"]
                    [(value % 6) as usize]
                    .to_owned(),
                _ => format!("{}", value % 100_000),
            },
        }
    }
}

impl TableProvider for LazyRows {
    fn column_count(&self) -> usize {
        COLUMNS
    }

    fn header(&self, index: usize) -> Option<Cow<'_, str>> {
        (index < COLUMNS).then(|| match index {
            0 => Cow::Borrowed("Row"),
            _ => Cow::Owned(format!("Column {index}")),
        })
    }

    fn headers(&self) -> HeaderIter<'_> {
        HeaderIter::new(self)
    }

    fn row_count(&self) -> usize {
        ROWS
    }

    fn is_lazy(&self) -> bool {
        true
    }

    /// Sorting or filtering a billion rows would read them all.
    fn can_sort(&self, _col: usize) -> bool {
        false
    }

    fn can_filter(&self, _col: usize) -> bool {
        false
    }

    fn prepare(&self, view: &Viewport<'_>) {
        let mut cache = self.cache.borrow_mut();
        for reply in self.replies.try_iter() {
            match reply {
                Reply::Loaded(block, values) => {
                    cache.pending.remove(&block);
                    cache.blocks.insert(block, values);
                    cache.touch(block);
                }
                Reply::Dropped(block) => {
                    cache.pending.remove(&block);
                }
            }
        }

        // The blocks in view, and the one after them. The loader serves the latest request first:
        // ask for the top of the view last.
        let Some(span) = view.rows.span() else {
            return;
        };
        let first = span.start() / BLOCK_ROWS;
        let last = (span.end() / BLOCK_ROWS + 1).min((ROWS - 1) / BLOCK_ROWS);
        for block in (first..=last).rev() {
            if cache.blocks.contains_key(&block) {
                cache.touch(block);
            } else if cache.pending.insert(block) && self.requests.send(block).is_err() {
                cache.pending.remove(&block);
            }
        }
        while cache.recent.len() > CACHED_BLOCKS {
            if let Some(oldest) = cache.recent.pop_front() {
                cache.blocks.remove(&oldest);
            }
        }
    }

    fn cell_status(&self, row: usize, col: usize) -> CellStatus<'_> {
        if !self.cache.borrow().blocks.contains_key(&(row / BLOCK_ROWS)) {
            CellStatus::Loading
        } else if Self::fails(row, col) {
            CellStatus::Failed(Cow::Borrowed("checksum mismatch in this page"))
        } else {
            CellStatus::Ready
        }
    }

    fn cell_at(&self, row: usize, col: usize) -> Result<Option<TableCell<'_>>, TableError> {
        let cache = self.cache.borrow();
        Ok(cache
            .blocks
            .get(&(row / BLOCK_ROWS))
            .and_then(|block| block.value(row, col))
            .map(|value| (Cow::Owned(Self::text(row, col, value)), None)))
    }

    fn for_all_rows(&self, _f: &mut RowCallback<'_>) -> Result<(), TableError> {
        Err(TableError::Operation(
            "a lazy table's rows can't all be walked".into(),
        ))
    }

    fn for_selected_rows(
        &self,
        state: &TableState,
        f: &mut RowCallback<'_>,
    ) -> Result<(), TableError> {
        for row_index in &state.selected_rows {
            f(&BorrowedRow {
                provider: self,
                row_index: row_index as usize,
            })?;
        }
        Ok(())
    }
}

struct LazyApp {
    rows: LazyRows,
    state: TableState,
    go_to: String,
}

impl LazyApp {
    fn new(ctx: &egui::Context) -> Self {
        let mut state = TableState::new("lazy", ROWS);
        state.selection_mode = SelectionMode::Explorer;
        Self {
            rows: LazyRows::new(ctx),
            state,
            go_to: String::new(),
        }
    }
}

impl eframe::App for LazyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            let mut scroll_to = None;
            ui.horizontal(|ui| {
                ui.heading("A billion rows");
                ui.separator();
                ui.label(self.state.counts_header(ROWS));
                ui.separator();
                let (loaded, loading) = {
                    let cache = self.rows.cache.borrow();
                    (cache.blocks.len(), cache.pending.len())
                };
                ui.label(format!("{loaded} blocks kept, {loading} loading"));
                ui.separator();
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.go_to)
                        .hint_text("Go to row")
                        .desired_width(120.0),
                );
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button("Go").clicked() || entered)
                    && let Ok(row) = self.go_to.trim().replace('_', "").parse::<u64>()
                {
                    scroll_to = Some(row.min(ROWS as u64 - 1));
                }
            });
            ui.add_space(4.0);

            let columns = (0..COLUMNS)
                .map(|col| {
                    Column::new(if col == 0 { 110.0 } else { 120.0 })
                        .range(40.0..=f32::INFINITY)
                        .resizable(true)
                })
                .collect();
            let mut table = TableKit::new("lazy", &self.rows, &mut self.state)
                .with_row_height(20.0)
                .with_cell_align(egui::Align::Center)
                .with_striped(true)
                .with_max_height(Some(ui.available_height()))
                .with_columns(columns);
            if let Some(row) = scroll_to {
                table = table.with_scroll_to_row(row, egui::Align::Center);
            }
            if let Err(error) = table.show(ui, |_, _, _, _| None) {
                ui.colored_label(ui.visuals().error_fg_color, error.to_string());
            }
        });
    }
}

fn main() -> eframe::Result {
    eframe::run_native(
        "Lazy table",
        eframe::NativeOptions::default(),
        Box::new(|cc| Ok(Box::new(LazyApp::new(&cc.egui_ctx)))),
    )
}
