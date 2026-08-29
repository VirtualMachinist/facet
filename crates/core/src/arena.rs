#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ArenaKey {
    pub(crate) slot: usize,
    pub(crate) generation: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Arena<T> {
    slots: Vec<ArenaSlot<T>>,
    free_head: Option<usize>,
    len: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free_head: None,
            len: 0,
        }
    }
}

impl<T> Arena<T> {
    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn insert(&mut self, value: T) -> ArenaKey {
        self.insert_with_key(|_| value)
    }

    pub(crate) fn insert_with_key(&mut self, value: impl FnOnce(ArenaKey) -> T) -> ArenaKey {
        self.len += 1;
        let key = if let Some(slot_index) = self.free_head {
            let slot = &mut self.slots[slot_index];
            self.free_head = slot.next_free.take();
            ArenaKey {
                slot: slot_index,
                generation: slot.generation,
            }
        } else {
            ArenaKey {
                slot: self.slots.len(),
                generation: 0,
            }
        };

        if key.slot == self.slots.len() {
            self.slots.push(ArenaSlot {
                generation: key.generation,
                value: Some(value(key)),
                next_free: None,
            });
        } else {
            self.slots[key.slot].value = Some(value(key));
        }
        key
    }

    pub(crate) fn get(&self, key: ArenaKey) -> Option<&T> {
        let slot = self.slots.get(key.slot)?;
        if slot.generation == key.generation {
            slot.value.as_ref()
        } else {
            None
        }
    }

    pub(crate) fn get_mut(&mut self, key: ArenaKey) -> Option<&mut T> {
        let slot = self.slots.get_mut(key.slot)?;
        if slot.generation == key.generation {
            slot.value.as_mut()
        } else {
            None
        }
    }

    pub(crate) fn remove(&mut self, key: ArenaKey) -> Option<T> {
        let slot = self.slots.get_mut(key.slot)?;
        if slot.generation != key.generation {
            return None;
        }

        let value = slot.value.take()?;
        if let Some(next_generation) = slot.generation.checked_add(1) {
            slot.generation = next_generation;
            slot.next_free = self.free_head;
            self.free_head = Some(key.slot);
        }
        self.len -= 1;
        Some(value)
    }

    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots.iter_mut().filter_map(|slot| slot.value.as_mut())
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &T> {
        self.slots.iter().filter_map(|slot| slot.value.as_ref())
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ArenaSlot<T> {
    generation: u64,
    value: Option<T>,
    next_free: Option<usize>,
}
