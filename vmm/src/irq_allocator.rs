pub struct IrqAllocator {
    next: u32,
}

impl IrqAllocator {
    pub fn new(start: u32) -> Self {
        Self { next: start }
    }

    pub fn allocate(&mut self) -> u32 {
        let irq = self.next;
        self.next = self.next.checked_add(1).expect("IRQ overflow");
        irq
    }

    pub fn peek(&self) -> u32 {
        self.next
    }
}

#[cfg(test)]
mod tests {
    use crate::irq_allocator::IrqAllocator;

    #[test]
    fn allocates_incrementing_irqs() {
        let mut alloc = IrqAllocator::new(32);
        assert_eq!(alloc.allocate(), 32);
        assert_eq!(alloc.allocate(), 33);
        assert_eq!(alloc.allocate(), 34);
    }

    #[test]
    fn peek_returns_next() {
        let mut alloc = IrqAllocator::new(10);
        assert_eq!(alloc.peek(), 10);
        alloc.allocate();
        assert_eq!(alloc.peek(), 11);
    }
}
