//! Highlighting layers powered by roaring bitmaps.

use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};

/// Highlighting structure mapping row indices to one of up to 10 color-tag groups.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Highlights(pub [RoaringBitmap; 10]);

impl Highlights {
    /// Accesses the highlight color index assigned to the row (if any).
    #[must_use]
    pub fn get_usize(&self, index: usize) -> Option<u8> {
        self.get(index as u32)
    }

    /// Adds a row index to a specific highlight color group.
    pub fn insert(&mut self, typ: u8, index: u32) -> bool {
        self.0
            .get_mut(typ as usize)
            .is_some_and(|bmp| bmp.insert(index))
    }

    /// Accesses the highlight color group index matching a specific row index.
    #[must_use]
    pub fn get(&self, index: u32) -> Option<u8> {
        self.0.iter().enumerate().find_map(|(i, bmp)| {
            if bmp.contains(index) {
                Some(i as u8)
            } else {
                None
            }
        })
    }

    /// Unions a selection map into a specific highlight group, removing them from other groups.
    pub fn insert_map(&mut self, typ: u8, map: &RoaringBitmap) {
        self.0.iter_mut().enumerate().for_each(|(i, bmp)| {
            if i as u8 == typ {
                *bmp |= map;
            } else {
                *bmp -= map;
            }
        });
    }

    /// Removes a collection of row indices from all color highlight groups.
    pub fn remove_map(&mut self, map: &RoaringBitmap) {
        for bmp in &mut self.0 {
            *bmp -= map;
        }
    }
}
