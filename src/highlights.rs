use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Highlights(pub [RoaringBitmap; 10]);

impl Highlights {
    #[must_use]
    pub fn get_usize(&self, index: usize) -> Option<u8> {
        self.get(index as u32)
    }

    pub fn insert(&mut self, typ: u8, index: u32) -> bool {
        self.0
            .get_mut(typ as usize)
            .is_some_and(|bmp| bmp.insert(index))
    }

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

    pub fn insert_map(&mut self, typ: u8, map: &RoaringBitmap) {
        self.0.iter_mut().enumerate().for_each(|(i, bmp)| {
            if i as u8 == typ {
                *bmp |= map;
            } else {
                *bmp -= map;
            }
        });
    }

    pub fn remove_map(&mut self, map: &RoaringBitmap) {
        for bmp in &mut self.0 {
            *bmp -= map;
        }
    }
}
