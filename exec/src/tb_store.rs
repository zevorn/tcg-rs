use tcg_backend::code_buffer::CodeBuffer;
use tcg_backend::HostCodeGen;
use tcg_core::tb::{TranslationBlock, TB_HASH_SIZE};

/// Storage and hash-table lookup for translation blocks.
///
/// Maps to QEMU's global TB hash table (`TBContext.htable`).
pub struct TbStore {
    tbs: Vec<TranslationBlock>,
    hash_buckets: Vec<Option<usize>>,
}

impl TbStore {
    pub fn new() -> Self {
        Self {
            tbs: Vec::with_capacity(1024),
            hash_buckets: vec![None; TB_HASH_SIZE],
        }
    }

    /// Allocate a new TB and return its index.
    pub fn alloc(&mut self, pc: u64, flags: u32, cflags: u32) -> usize {
        let idx = self.tbs.len();
        self.tbs.push(TranslationBlock::new(pc, flags, cflags));
        idx
    }

    /// Lookup a valid TB by (pc, flags) in the hash table.
    pub fn lookup(&self, pc: u64, flags: u32) -> Option<usize> {
        let bucket = TranslationBlock::hash(pc, flags);
        let mut cur = self.hash_buckets[bucket];
        while let Some(idx) = cur {
            let tb = &self.tbs[idx];
            if !tb.invalid && tb.pc == pc && tb.flags == flags {
                return Some(idx);
            }
            cur = tb.hash_next;
        }
        None
    }

    /// Insert a TB into the hash table (prepend to bucket).
    pub fn insert(&mut self, tb_idx: usize) {
        let pc = self.tbs[tb_idx].pc;
        let flags = self.tbs[tb_idx].flags;
        let bucket = TranslationBlock::hash(pc, flags);
        self.tbs[tb_idx].hash_next = self.hash_buckets[bucket];
        self.hash_buckets[bucket] = Some(tb_idx);
    }

    pub fn get(&self, idx: usize) -> &TranslationBlock {
        &self.tbs[idx]
    }

    pub fn get_mut(&mut self, idx: usize) -> &mut TranslationBlock {
        &mut self.tbs[idx]
    }

    /// Mark a TB as invalid, unlink all chained jumps, and
    /// remove it from the hash chain.
    pub fn invalidate<B: HostCodeGen>(
        &mut self,
        tb_idx: usize,
        code_buf: &mut CodeBuffer,
        backend: &mut B,
    ) {
        self.tbs[tb_idx].invalid = true;

        // 1. Unlink incoming edges: reset each source TB's
        //    goto_tb jump back to its epilogue fallback.
        let jmp_list = std::mem::take(&mut self.tbs[tb_idx].jmp_list);
        for (src, slot) in jmp_list {
            Self::reset_jump(&self.tbs[src], code_buf, backend, slot);
            self.tbs[src].jmp_dest[slot] = None;
        }

        // 2. Unlink outgoing edges: remove ourselves from
        //    each destination TB's jmp_list.
        for slot in 0..2 {
            if let Some(dst) = self.tbs[tb_idx].jmp_dest[slot].take() {
                self.tbs[dst]
                    .jmp_list
                    .retain(|&(s, n)| !(s == tb_idx && n == slot));
            }
        }

        // 3. Remove from hash chain.
        let pc = self.tbs[tb_idx].pc;
        let flags = self.tbs[tb_idx].flags;
        let bucket = TranslationBlock::hash(pc, flags);
        let mut prev: Option<usize> = None;
        let mut cur = self.hash_buckets[bucket];
        while let Some(idx) = cur {
            if idx == tb_idx {
                let next = self.tbs[idx].hash_next;
                if let Some(p) = prev {
                    self.tbs[p].hash_next = next;
                } else {
                    self.hash_buckets[bucket] = next;
                }
                self.tbs[idx].hash_next = None;
                return;
            }
            prev = cur;
            cur = self.tbs[idx].hash_next;
        }
    }

    /// Reset a goto_tb jump back to its original (epilogue)
    /// target, effectively unlinking the direct chain.
    fn reset_jump<B: HostCodeGen>(
        tb: &TranslationBlock,
        code_buf: &mut CodeBuffer,
        backend: &mut B,
        slot: usize,
    ) {
        if let (Some(jmp_off), Some(reset_off)) =
            (tb.jmp_insn_offset[slot], tb.jmp_reset_offset[slot])
        {
            // Offsets are absolute code buffer positions.
            backend.patch_jump(code_buf, jmp_off as usize, reset_off as usize);
        }
    }

    /// Flush all TBs and reset the hash table.
    pub fn flush(&mut self) {
        self.tbs.clear();
        self.hash_buckets.fill(None);
    }

    pub fn len(&self) -> usize {
        self.tbs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tbs.is_empty()
    }
}

impl Default for TbStore {
    fn default() -> Self {
        Self::new()
    }
}
