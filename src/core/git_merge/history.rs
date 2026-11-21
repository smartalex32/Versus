use super::ConflictState;

#[derive(Clone, Debug)]
pub(super) struct Snapshot {
    pub result: String,
    pub conflicts: Vec<ConflictState>,
}

impl Snapshot {
    fn byte_size(&self) -> usize {
        self.result.len() + self.conflicts.len() * std::mem::size_of::<ConflictState>()
    }
}

#[derive(Debug)]
pub(super) struct History {
    entries: Vec<Snapshot>,
    bytes: usize,
    current: usize,
}

impl History {
    const MAX_ENTRIES: usize = 100;

    pub(super) fn new(initial: Snapshot) -> Self {
        let bytes = initial.byte_size();
        Self {
            entries: vec![initial],
            current: 0,
            bytes,
        }
    }

    pub(super) fn push(&mut self, snapshot: Snapshot) {
        self.entries.truncate(self.current + 1);
        self.bytes = self.entries.iter().map(|entry| entry.byte_size()).sum();
        self.bytes += snapshot.byte_size();
        self.entries.push(snapshot);
        self.current += 1;
        while self.entries.len() > Self::MAX_ENTRIES || self.bytes > 128 * 1024 * 1024 {
            if self.entries.len() == 1 {
                break;
            }
            let removed = self.entries.remove(0);
            self.bytes -= removed.byte_size();
            self.current -= 1;
        }
    }

    pub(super) fn undo(&mut self) -> Option<Snapshot> {
        (self.current > 0).then(|| {
            self.current -= 1;
            self.entries[self.current].clone()
        })
    }

    pub(super) fn redo(&mut self) -> Option<Snapshot> {
        (self.current + 1 < self.entries.len()).then(|| {
            self.current += 1;
            self.entries[self.current].clone()
        })
    }

    pub(super) fn can_undo(&self) -> bool {
        self.current > 0
    }
    pub(super) fn can_redo(&self) -> bool {
        self.current + 1 < self.entries.len()
    }
}
