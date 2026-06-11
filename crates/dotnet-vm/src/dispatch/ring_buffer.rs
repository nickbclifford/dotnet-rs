use gc_arena::static_collect;

#[derive(Default, Clone)]
pub struct InstructionRingBuffer {
    // Store (IP, opcode-name) — a &'static str returned by Instruction::name().
    // Storing the full Instruction enum was expensive: each push cloned owning heap
    // data (UserMethod, MethodType, etc.) and dropping 10 of them on ExecutionEngine
    // teardown showed up at 9% self-time in profiles. The name is a zero-cost
    // &'static str and gives readable crash output without any allocation.
    buffer: [Option<(usize, &'static str)>; 10],
    index: usize,
}

static_collect!(InstructionRingBuffer);

impl InstructionRingBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, ip: usize, name: &'static str) {
        self.buffer[self.index] = Some((ip, name));
        self.index = (self.index + 1) % 10;
    }

    pub fn dump(&self) -> String {
        let mut res = String::new();
        for i in 0..10 {
            let idx = (self.index + i) % 10;
            if let Some((ip, name)) = self.buffer[idx] {
                res.push_str(&format!("[{ip:04x}] {name}\n"));
            }
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_wrapping() {
        let mut buffer = InstructionRingBuffer::new();
        for i in 0..15 {
            buffer.push(i, "Add");
        }

        let dump = buffer.dump();
        let lines: Vec<_> = dump.lines().collect();
        assert_eq!(lines.len(), 10);
        for line in lines {
            assert!(line.contains("Add"));
        }
    }
}
