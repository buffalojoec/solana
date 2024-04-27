use {
    crate::compute_budget::ComputeBudget, solana_sdk::instruction::InstructionError,
    std::cell::RefCell,
};

pub(crate) struct ComputeMeter {
    budget: ComputeBudget,
    current_budget: ComputeBudget,
    meter: RefCell<u64>,
}

impl ComputeMeter {
    pub(crate) fn new(budget: ComputeBudget) -> Self {
        Self {
            budget,
            current_budget: budget,
            meter: RefCell::new(budget.compute_unit_limit),
        }
    }

    pub(crate) fn consume(&mut self, amount: u64) {
        // 1 to 1 instruction to compute unit mapping
        // ignore overflow, Ebpf will bail if exceeded
        let mut meter = self.meter.borrow_mut();
        *meter = meter.saturating_sub(amount);
    }

    pub(crate) fn consume_checked(&self, amount: u64) -> Result<(), Box<dyn std::error::Error>> {
        let mut meter = self.meter.borrow_mut();
        let exceeded = *meter < amount;
        *meter = meter.saturating_sub(amount);
        if exceeded {
            return Err(Box::new(InstructionError::ComputationalBudgetExceeded));
        }
        Ok(())
    }

    pub(crate) fn get_remaining(&self) -> u64 {
        *self.meter.borrow()
    }

    pub(crate) fn mock_set_remaining(&self, remaining: u64) {
        *self.meter.borrow_mut() = remaining;
    }

    pub(crate) fn get_current_budget(&self) -> &ComputeBudget {
        &self.current_budget
    }

    pub(crate) fn update_current_budget(&mut self, budget: ComputeBudget) {
        self.current_budget = budget;
    }
}
