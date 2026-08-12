use {
    crate::invoke_context::{BpfAllocator, InvokeContext},
    solana_instruction::error::InstructionError,
    solana_sbpf::{
        ebpf::{MM_BYTECODE_START, MM_HEAP_START, MM_REGION_SIZE, MM_RODATA_START, MM_STACK_START},
        elf::Executable,
        memory_region::{AccessViolationHandler, MemoryMapping, MemoryRegion},
        program::SBPFVersion,
        vm::{Config, ContextObject},
    },
    solana_sysvar_id::SysvarId,
    solana_transaction_context::{
        IndexOfAccount,
        transaction::TransactionContext,
        vm_addresses::{
            self, ACCOUNT_METADATA_AREA, GUEST_ACCOUNT_PAYLOAD_BASE_ADDRESS,
            GUEST_INSTRUCTION_ACCOUNT_BASE_ADDRESS, GUEST_INSTRUCTION_ACCOUNT_END_ADDRESS,
            GUEST_INSTRUCTION_DATA_BASE_ADDRESS, GUEST_SYSVARS_BASE_ADDRESS,
            GUEST_SYSVARS_END_ADDRESS, INSTRUCTION_TRACE_AREA, RETURN_DATA_SCRATCHPAD,
            TRANSACTION_FRAME_ADDRESS, abiv2_region_index_from_vm_address,
        },
    },
};

enum MemoryContextType {
    ABIv1(MemoryContext),
    Placeholder,
    ABIv2,
}

pub struct MemoryContexts {
    contexts: Vec<MemoryContextType>,
    abiv2_mappings: Box<MemoryMapping>,
}

impl MemoryContexts {
    pub(crate) fn new() -> Self {
        Self {
            contexts: Vec::new(),
            abiv2_mappings: Box::new(unsafe {
                MemoryMapping::new(Vec::new(), &Config::default(), SBPFVersion::Reserved).unwrap()
            }),
        }
    }

    /// Set this instruction's [`MemoryContext`].
    pub fn set_memory_context_abi_v1(
        &mut self,
        memory_context: MemoryContext,
    ) -> Result<(), InstructionError> {
        *self
            .contexts
            .last_mut()
            .ok_or(InstructionError::CallDepth)? = MemoryContextType::ABIv1(memory_context);
        Ok(())
    }

    /// Get current instruction's [`MemoryContext`]
    pub fn memory_context_abi_v1(&self) -> Result<&MemoryContext, InstructionError> {
        match self.contexts.last().ok_or(InstructionError::CallDepth)? {
            MemoryContextType::ABIv1(ctx) => Ok(ctx),
            MemoryContextType::Placeholder => Err(InstructionError::ProgramEnvironmentSetupFailure),
            MemoryContextType::ABIv2 => Err(InstructionError::InvalidAccountData),
        }
    }

    /// Get current instruction's [`MemoryContext`] for mutable use.
    pub fn memory_context_mut_abi_v1(&mut self) -> Result<&mut MemoryContext, InstructionError> {
        let context = self
            .contexts
            .last_mut()
            .ok_or(InstructionError::CallDepth)?;

        match context {
            MemoryContextType::ABIv1(ctx) => Ok(ctx),
            MemoryContextType::Placeholder => Err(InstructionError::ProgramEnvironmentSetupFailure),
            MemoryContextType::ABIv2 => Err(InstructionError::ProgramEnvironmentSetupFailure),
        }
    }

    pub fn memory_mapping(&self) -> Result<&MemoryMapping, InstructionError> {
        let mapping = match self.contexts.last().ok_or(InstructionError::CallDepth)? {
            MemoryContextType::ABIv1(ctx) => &ctx.memory_mapping,
            MemoryContextType::Placeholder => {
                return Err(InstructionError::ProgramEnvironmentSetupFailure);
            }
            MemoryContextType::ABIv2 => &self.abiv2_mappings,
        };

        Ok(mapping)
    }

    pub fn memory_mapping_mut(&mut self) -> Result<&mut MemoryMapping, InstructionError> {
        let mapping = match self
            .contexts
            .last_mut()
            .ok_or(InstructionError::CallDepth)?
        {
            MemoryContextType::ABIv1(ctx) => &mut ctx.memory_mapping,
            MemoryContextType::Placeholder => {
                return Err(InstructionError::ProgramEnvironmentSetupFailure);
            }
            MemoryContextType::ABIv2 => &mut self.abiv2_mappings,
        };

        Ok(mapping)
    }

    #[cfg(feature = "dev-context-only-utils")]
    pub fn mock_set_mapping_abi_v1(&mut self, memory_mapping: MemoryMapping) {
        self.contexts = vec![MemoryContextType::ABIv1(MemoryContext {
            allocator: BpfAllocator::new(0),
            accounts_metadata: vec![],
            memory_mapping: Box::new(memory_mapping),
        })];
    }

    #[cfg(feature = "dev-context-only-utils")]
    pub fn mock_set_mapping_abi_v2(&mut self, memory_mapping: MemoryMapping) {
        *self.abiv2_mappings = memory_mapping;
        self.contexts = vec![MemoryContextType::ABIv2];
    }

    pub fn push(&mut self, transaction_context: &TransactionContext) {
        // We are only pushing a placeholder to be configured later
        self.contexts.push(MemoryContextType::Placeholder);
        if self.abi_v2_regions_exist() {
            // The instruction trace length has changed so we need to update the
            // underlying mapping
            let regions = self.abiv2_mappings.get_regions_mut();
            let trace_index = abiv2_region_index_from_vm_address(INSTRUCTION_TRACE_AREA);
            *regions
                .get_mut(trace_index)
                .expect("Instruction trace region must exist") = MemoryRegion::new(
                transaction_context.instruction_trace_as_raw_slice(),
                INSTRUCTION_TRACE_AREA,
            );
        }
    }

    pub fn pop(&mut self) {
        self.contexts.pop();
    }

    pub fn abi_v2_regions_exist(&self) -> bool {
        !self.abiv2_mappings.get_regions().is_empty()
    }

    pub fn create_abi_v2_mappings<C: ContextObject>(
        &mut self,
        regions: Vec<MemoryRegion>,
        executable: &Executable<C>,
        access_violation_handler: AccessViolationHandler,
    ) {
        *self.abiv2_mappings = unsafe {
            MemoryMapping::new_uninitialized(
                regions,
                executable.get_config(),
                executable.get_sbpf_version(),
                access_violation_handler,
            )
        };
    }

    pub fn set_abi_v2(&mut self) -> Result<(), InstructionError> {
        *self
            .contexts
            .last_mut()
            .ok_or(InstructionError::CallDepth)? = MemoryContextType::ABIv2;
        Ok(())
    }

    /// Returns if an ABIv2 instruction is the parent of this one
    ///
    /// This function can only be called after `MemoryContexts::push` was invoked,
    /// and the placeholder has been replaced by either an ABIv1 or v2 memory context.
    pub fn is_parent_abi_v2_instruction(&self) -> Result<bool, InstructionError> {
        Ok(matches!(
            self.contexts
                .get(self.contexts.len().saturating_sub(2))
                .ok_or(InstructionError::CallDepth)?,
            MemoryContextType::ABIv2
        ))
    }

    /// Modifies the memory regions as needed between any instruction edges.
    ///
    /// This function is to be called before execution changes between instructions: before a new
    /// program is executed, after a CPI return, etc.
    ///
    /// When returning from a CPI, we must also reconfigure the account permissions. If the CPI
    /// was made into an ABIv1 program, the underlying pointer may have changed, so `check_pointers = true`
    pub fn abi_v2_prepare_for_instruction(
        &mut self,
        transaction_context: &TransactionContext,
        check_pointers: bool,
    ) -> Result<(), InstructionError> {
        let current_instruction = transaction_context.get_current_instruction_context()?;
        let regions = self.abiv2_mappings.get_regions_mut();

        // Before using the scratchpad the instruction has to call set_buffer_length syscall.
        // This is required in order to set the `program_id`.
        let return_data_scratchpad = regions
            .get_mut(abiv2_region_index_from_vm_address(RETURN_DATA_SCRATCHPAD))
            .expect("return data scratchpad always present");
        return_data_scratchpad.make_immutable();

        let accounts_in_transaction = transaction_context.accounts().len();
        let accounts_start = abiv2_region_index_from_vm_address(GUEST_ACCOUNT_PAYLOAD_BASE_ADDRESS);
        let accounts_end = accounts_start.saturating_add(accounts_in_transaction);
        let account_regions = regions
            .get_mut(accounts_start..accounts_end)
            .expect("Account regions should have been configured.");

        for (tx_idx, acc_region) in account_regions.iter_mut().enumerate() {
            if let Ok(idx_in_ix) =
                current_instruction.get_index_of_account_in_instruction(tx_idx as IndexOfAccount)
            {
                let borrowed_account =
                    current_instruction.try_borrow_instruction_account(idx_in_ix)?;

                // If an account has been reallocated in ABIv1, we must update the backing
                // pointer
                if check_pointers
                    && !core::ptr::eq(
                        borrowed_account.get_data().as_ptr(),
                        acc_region.host_buffer().ptr() as *const u8,
                    )
                {
                    // SAFETY: The new underlying memory is backed by a valid pointer
                    unsafe {
                        acc_region.redirect(&raw const borrowed_account.get_data()[..]);
                    }
                }

                let can_data_be_changed = borrowed_account.can_data_be_changed();
                if can_data_be_changed.is_ok() && !acc_region.host_buffer().is_mutable() {
                    acc_region.access_violation_handler_payload = Some(tx_idx as IndexOfAccount);
                } else if can_data_be_changed.is_err() {
                    acc_region.access_violation_handler_payload = None;
                    acc_region.make_immutable();
                }
            } else {
                acc_region.access_violation_handler_payload = None;
                acc_region.make_immutable();
            }
        }

        Ok(())
    }

    /// Update the backing memory for return data, when returning from an ABIv1 program
    pub fn abi_v2_update_return_data(&mut self, transaction_context: &mut TransactionContext) {
        let regions = self.abiv2_mappings.get_regions_mut();
        let return_data_scratchpad_index =
            abiv2_region_index_from_vm_address(RETURN_DATA_SCRATCHPAD);
        let return_data_region = regions
            .get_mut(return_data_scratchpad_index)
            .expect("Return data scratchpad must exist");

        *return_data_region = transaction_context.return_data_region();
    }

    /// When configuring CPI from an ABIv1 program, manually update the instruction
    /// area regions.
    pub fn prepare_instruction_regions(
        &mut self,
        transaction_context: &mut TransactionContext,
        instruction_index: usize,
    ) {
        // ABIv2 regions are lazily initialized. If the transaction has not yet executed an ABIv2
        // instruction, the regions do not exist and do not need to be prepared. The creation,
        // when it happens in `create_abiv2_regions` will already have the updated values.
        if !self.abi_v2_regions_exist() {
            return;
        }

        let regions = self.abiv2_mappings.get_regions_mut();
        let ix_ctx = transaction_context
            .get_instruction_context_at_index_in_trace(instruction_index)
            .expect("Instruction must exist");

        // Configure instruction data
        let data_vm_address = GUEST_INSTRUCTION_DATA_BASE_ADDRESS
            .saturating_add(MM_REGION_SIZE.saturating_mul(instruction_index as u64));
        let instruction_data_index = abiv2_region_index_from_vm_address(data_vm_address);

        *regions
            .get_mut(instruction_data_index)
            .expect("Region must have been pre configured") = MemoryRegion::new(
            &raw const ix_ctx.get_instruction_data()[..],
            data_vm_address,
        );

        // Configure instruction accounts
        let accounts_vm_address = GUEST_INSTRUCTION_ACCOUNT_BASE_ADDRESS
            .saturating_add(MM_REGION_SIZE.saturating_mul(instruction_index as u64));
        let instruction_accounts_index = abiv2_region_index_from_vm_address(accounts_vm_address);
        *regions
            .get_mut(instruction_accounts_index)
            .expect("Region must have been pre configured") = MemoryRegion::new(
            &raw const ix_ctx.instruction_accounts()[..],
            accounts_vm_address,
        );
    }
}

/// This structure contains metadata about the memory for each instruction under execution.
/// The BpfAllocator, accounts addresses in the guest and the memory mapping.
pub struct MemoryContext {
    pub allocator: BpfAllocator,
    pub accounts_metadata: Vec<SerializedAccountMetadata>,
    memory_mapping: Box<MemoryMapping>,
}

impl MemoryContext {
    /// Creates a new memory context
    pub fn new(
        allocator: BpfAllocator,
        accounts_metadata: Vec<SerializedAccountMetadata>,
        memory_mapping: MemoryMapping,
    ) -> Self {
        Self {
            allocator,
            accounts_metadata,
            memory_mapping: Box::new(memory_mapping),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SerializedAccountMetadata {
    /// Address of the first byte of the serialized account record (the
    /// `NON_DUP_MARKER`/duplicate-marker byte).
    pub vm_addr: u64,
    pub original_data_len: usize,
    pub vm_data_addr: u64,
    pub vm_key_addr: u64,
    pub vm_lamports_addr: u64,
    pub vm_owner_addr: u64,
}

#[cfg_attr(feature = "dev-context-only-utils", qualifier_attr::qualifiers(pub))]
pub(crate) fn create_abiv2_regions(invoke_context: &mut InvokeContext) -> Vec<MemoryRegion> {
    const NUMBER_OF_REGIONS: usize =
        vm_addresses::abiv2_region_index_from_vm_address(GUEST_INSTRUCTION_ACCOUNT_END_ADDRESS);
    let mut v2_regions: Vec<MemoryRegion> = Vec::with_capacity(NUMBER_OF_REGIONS);
    let InvokeContext {
        transaction_context,
        environment_config,
        ..
    } = invoke_context;

    // Filled on a later stage, but we still want to have at least base vm_addrs be accurate so that
    // there are no duplicate regions (for e.g. tests.)
    for vm_addr in [
        MM_RODATA_START,
        MM_BYTECODE_START,
        MM_STACK_START,
        MM_HEAP_START,
    ] {
        v2_regions.push(MemoryRegion::new_empty(vm_addr));
    }

    let transaction_frame_region = MemoryRegion::new(
        transaction_context.transaction_frame_address(),
        TRANSACTION_FRAME_ADDRESS,
    );
    v2_regions.push(transaction_frame_region);

    let accounts_slice = transaction_context.accounts().shared_fields_as_raw_slice();
    v2_regions.push(MemoryRegion::new(accounts_slice, ACCOUNT_METADATA_AREA));

    let instruction_trace_slice = transaction_context.instruction_trace_as_raw_slice();
    v2_regions.push(MemoryRegion::new(
        instruction_trace_slice,
        INSTRUCTION_TRACE_AREA,
    ));
    v2_regions.push(transaction_context.return_data_region());
    v2_regions.extend(transaction_context.accounts().account_payload_regions());

    // NOTE: there are padding regions between accounts and sysvars which are populated (if needed)
    // during construction of `MemoryMapping`.
    let start_idx = abiv2_region_index_from_vm_address(GUEST_SYSVARS_BASE_ADDRESS);
    let end_idx = abiv2_region_index_from_vm_address(GUEST_SYSVARS_END_ADDRESS);
    let sysvars = environment_config.sysvar_cache();
    let sysvar_ids = [
        solana_clock::Clock::id(),
        solana_epoch_rewards::EpochRewards::id(),
        solana_epoch_schedule::EpochSchedule::id(),
        solana_last_restart_slot::LastRestartSlot::id(),
        solana_rent::Rent::id(),
        solana_slot_hashes::SlotHashes::id(),
        solana_stake_history::StakeHistory::id(),
    ];
    for (idx, var) in (start_idx..end_idx).zip(sysvar_ids.into_iter().rev()) {
        let data = sysvars.sysvar_id_to_buffer(&var).as_deref();
        let data = data.unwrap_or_default();
        v2_regions.push(MemoryRegion::new(
            &raw const data[..],
            vm_addresses::from_index(idx as u64),
        ));
    }

    v2_regions.extend(transaction_context.instruction_payload_regions());
    v2_regions.extend(transaction_context.instruction_accounts_regions());

    v2_regions
}

#[cfg(test)]
mod test {
    use {
        crate::{
            memory_context::{MemoryContexts, create_abiv2_regions},
            with_mock_invoke_context,
        },
        solana_account::AccountSharedData,
        solana_pubkey::Pubkey,
        solana_sbpf::{
            memory_region::{MemoryMapping, default_access_violation_handler},
            program::SBPFVersion,
            vm::Config,
        },
        solana_transaction_context::{
            instruction_accounts::InstructionAccount,
            vm_addresses::{
                GUEST_ACCOUNT_PAYLOAD_BASE_ADDRESS, abiv2_region_index_from_vm_address,
            },
        },
    };

    #[test]
    fn test_update_account_permissions() {
        let program = Pubkey::new_unique();
        let accounts = vec![
            (
                Pubkey::new_unique(),
                AccountSharedData::new(20, 10, &program),
            ),
            (
                Pubkey::new_unique(),
                AccountSharedData::new(30, 15, &program),
            ),
            (
                Pubkey::new_unique(),
                AccountSharedData::new(40, 5, &program),
            ),
            (
                Pubkey::new_unique(),
                AccountSharedData::new(60, 2, &program),
            ),
            (
                program,
                AccountSharedData::new(20, 3, &Pubkey::new_unique()),
            ),
        ];
        with_mock_invoke_context!(invoke_context, _temp_tx_context, Vec::new());
        let mut tx_context = TransactionContext::new(accounts, Rent::default(), 4, 64, 3);
        invoke_context.transaction_context = &mut tx_context;

        invoke_context
            .transaction_context
            .configure_top_level_instruction_for_tests(
                4,
                vec![
                    InstructionAccount::new(0, false, false),
                    InstructionAccount::new(2, false, true),
                    InstructionAccount::new(1, false, false),
                    InstructionAccount::new(3, false, true),
                ],
                Vec::new(),
            )
            .unwrap();

        let mut memory_contexts = MemoryContexts::new();
        let abi_v2_regions = create_abiv2_regions(&mut invoke_context);
        *memory_contexts.abiv2_mappings = unsafe {
            MemoryMapping::new_uninitialized(
                abi_v2_regions,
                &Config::default(),
                SBPFVersion::V4,
                Box::new(default_access_violation_handler),
            )
        };

        let start = abiv2_region_index_from_vm_address(GUEST_ACCOUNT_PAYLOAD_BASE_ADDRESS);

        // IX 1
        tx_context.push().unwrap();
        memory_contexts
            .abi_v2_prepare_for_instruction(&tx_context, false)
            .unwrap();
        let end = start.saturating_add(tx_context.accounts().len());
        let ix1_regions = memory_contexts
            .abiv2_mappings
            .get_regions()
            .get(start..end)
            .unwrap();

        let reg_zero = ix1_regions.first().unwrap();
        assert!(reg_zero.access_violation_handler_payload.is_none());
        assert!(!reg_zero.host_buffer().is_mutable());
        let reg_one = ix1_regions.get(1).unwrap();
        assert!(reg_one.access_violation_handler_payload.is_none());
        assert!(!reg_one.host_buffer().is_mutable());
        let reg_two = ix1_regions.get(2).unwrap();
        assert_eq!(reg_two.access_violation_handler_payload, Some(2));
        assert!(!reg_two.host_buffer().is_mutable());
        let reg_three = ix1_regions.get(3).unwrap();
        assert_eq!(reg_three.access_violation_handler_payload, Some(3));
        assert!(!reg_three.host_buffer().is_mutable());
        for account_region in ix1_regions.iter().skip(4) {
            assert!(account_region.access_violation_handler_payload.is_none());
        }

        // IX 2
        tx_context.pop().unwrap();

        tx_context
            .configure_top_level_instruction_for_tests(
                4,
                vec![
                    InstructionAccount::new(1, false, false),
                    InstructionAccount::new(2, false, false),
                    InstructionAccount::new(0, false, true),
                ],
                Vec::new(),
            )
            .unwrap();
        tx_context.push().unwrap();

        memory_contexts
            .abi_v2_prepare_for_instruction(&tx_context, false)
            .unwrap();
        let end = start.saturating_add(tx_context.accounts().len());
        let ix2_regions = memory_contexts
            .abiv2_mappings
            .get_regions()
            .get(start..end)
            .unwrap();

        let reg_zero = ix2_regions.first().unwrap();
        assert_eq!(reg_zero.access_violation_handler_payload, Some(0));
        assert!(!reg_zero.host_buffer().is_mutable());
        let reg_one = ix2_regions.get(1).unwrap();
        assert!(reg_one.access_violation_handler_payload.is_none());
        assert!(!reg_one.host_buffer().is_mutable());
        let reg_two = ix2_regions.get(2).unwrap();
        assert!(reg_two.access_violation_handler_payload.is_none());
        assert!(!reg_two.host_buffer().is_mutable());
        let reg_three = ix2_regions.get(3).unwrap();
        assert!(reg_three.access_violation_handler_payload.is_none());
        assert!(!reg_three.host_buffer().is_mutable());
        for account_region in ix2_regions.iter().skip(4) {
            assert!(account_region.access_violation_handler_payload.is_none());
        }

        // IX 3
        tx_context.pop().unwrap();

        tx_context
            .configure_top_level_instruction_for_tests(
                4,
                vec![
                    InstructionAccount::new(0, false, true),
                    InstructionAccount::new(1, false, true),
                    InstructionAccount::new(2, false, false),
                ],
                Vec::new(),
            )
            .unwrap();

        tx_context.push().unwrap();
        memory_contexts
            .abi_v2_prepare_for_instruction(&tx_context, false)
            .unwrap();
        let end = start.saturating_add(tx_context.accounts().len());
        let ix3_regions = memory_contexts
            .abiv2_mappings
            .get_regions()
            .get(start..end)
            .unwrap();
        let reg_zero = ix3_regions.first().unwrap();
        assert_eq!(reg_zero.access_violation_handler_payload, Some(0));
        assert!(!reg_zero.host_buffer().is_mutable());
        let reg_one = ix3_regions.get(1).unwrap();
        assert_eq!(reg_one.access_violation_handler_payload, Some(1));
        assert!(!reg_one.host_buffer().is_mutable());
        let reg_two = ix3_regions.get(2).unwrap();
        assert!(reg_two.access_violation_handler_payload.is_none());
        assert!(!reg_two.host_buffer().is_mutable());
        for account_region in ix3_regions.iter().skip(3) {
            assert!(account_region.access_violation_handler_payload.is_none());
        }

        // IX 3 again, but with region made writable
        let end = start.saturating_add(tx_context.accounts().len());
        let first_account = memory_contexts
            .abiv2_mappings
            .get_regions_mut()
            .get_mut(start..end)
            .unwrap()
            .first_mut()
            .unwrap();
        unsafe {
            first_account.redirect(first_account.host_buffer().mutable());
        }
        first_account.access_violation_handler_payload = None;
        memory_contexts
            .abi_v2_prepare_for_instruction(&tx_context, false)
            .unwrap();
        let end = start.saturating_add(tx_context.accounts().len());
        let ix3_regions = memory_contexts
            .abiv2_mappings
            .get_regions()
            .get(start..end)
            .unwrap();
        let reg_zero = ix3_regions.first().unwrap();
        assert!(reg_zero.access_violation_handler_payload.is_none());
        assert!(reg_zero.host_buffer().is_mutable());
        let reg_one = ix3_regions.get(1).unwrap();
        assert_eq!(reg_one.access_violation_handler_payload, Some(1));
        assert!(!reg_one.host_buffer().is_mutable());
        let reg_two = ix3_regions.get(2).unwrap();
        assert!(reg_two.access_violation_handler_payload.is_none());
        assert!(!reg_two.host_buffer().is_mutable());
        for account_region in ix3_regions.iter().skip(3) {
            assert!(account_region.access_violation_handler_payload.is_none());
        }
    }
}
