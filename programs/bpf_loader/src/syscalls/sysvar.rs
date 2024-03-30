use super::*;

declare_builtin_function!(
    /// Get a slice of sysvar data from the sysvar cache.
    SyscallGetSysvar,
    fn rust(
        invoke_context: &mut InvokeContext,
        sysvar_id: u64,
        var_addr: u64,
        offset: u64,
        length: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        let compute_budget = invoke_context.get_compute_budget();
        let div_by_cpi = |n: u64| -> u64 {
            n.checked_div(compute_budget.cpi_bytes_per_unit)
                .unwrap_or(u64::MAX)
        };
        consume_compute_meter(
            invoke_context,
            compute_budget
                .sysvar_base_cost
                .max(div_by_cpi(32).saturating_add(div_by_cpi(length)))
        )?;

        let check_aligned = invoke_context.get_check_aligned();
        let sysvar_cache = invoke_context.get_sysvar_cache();

        let sysvar_id = translate_type::<Pubkey>(memory_mapping, sysvar_id, check_aligned)?;
        let sysvar_bytes = match sysvar_cache.get_sysvar_bytes(sysvar_id) {
            Ok(bytes) => bytes,
            Err(err) => match err {
                // The sysvar was unavailable in the sysvar cache, return `1`.
                InstructionError::UnsupportedSysvar => return Ok(1),
                // Unknown error
                _ => return Err(err.into()),
            }
        };

        let offset = offset as usize;
        let end = offset
            .checked_add(length as usize)
            .ok_or(InstructionError::ArithmeticOverflow)?;
        if end > sysvar_bytes.len() {
            // The `offset` and `length` provided are out of range for the
            // sysvar data, return `2`.
            return Ok(2);
        }

        let var = translate_slice_mut::<u8>(memory_mapping, var_addr, length, check_aligned)?;
        var.copy_from_slice(
            sysvar_bytes
            .get(offset..end)
            .ok_or(InstructionError::InvalidArgument)?,
        );

        Ok(SUCCESS)
    }
);

fn get_sysvar<T: std::fmt::Debug + Sysvar + SysvarId + Clone>(
    sysvar: Result<Arc<T>, InstructionError>,
    var_addr: u64,
    check_aligned: bool,
    memory_mapping: &mut MemoryMapping,
    invoke_context: &mut InvokeContext,
) -> Result<u64, Error> {
    consume_compute_meter(
        invoke_context,
        invoke_context
            .get_compute_budget()
            .sysvar_base_cost
            .saturating_add(size_of::<T>() as u64),
    )?;
    let var = translate_type_mut::<T>(memory_mapping, var_addr, check_aligned)?;

    let sysvar: Arc<T> = sysvar?;
    *var = T::clone(sysvar.as_ref());

    Ok(SUCCESS)
}

declare_builtin_function!(
    /// Get a Clock sysvar
    SyscallGetClockSysvar,
    fn rust(
        invoke_context: &mut InvokeContext,
        var_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        get_sysvar(
            invoke_context.get_sysvar_cache().get_clock(),
            var_addr,
            invoke_context.get_check_aligned(),
            memory_mapping,
            invoke_context,
        )
    }
);

declare_builtin_function!(
    /// Get a EpochSchedule sysvar
    SyscallGetEpochScheduleSysvar,
    fn rust(
        invoke_context: &mut InvokeContext,
        var_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        get_sysvar(
            invoke_context.get_sysvar_cache().get_epoch_schedule(),
            var_addr,
            invoke_context.get_check_aligned(),
            memory_mapping,
            invoke_context,
        )
    }
);

declare_builtin_function!(
    /// Get a EpochRewards sysvar
    SyscallGetEpochRewardsSysvar,
    fn rust(
        invoke_context: &mut InvokeContext,
        var_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        get_sysvar(
            invoke_context.get_sysvar_cache().get_epoch_rewards(),
            var_addr,
            invoke_context.get_check_aligned(),
            memory_mapping,
            invoke_context,
        )
    }
);

declare_builtin_function!(
    /// Get a Fees sysvar
    SyscallGetFeesSysvar,
    fn rust(
        invoke_context: &mut InvokeContext,
        var_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        #[allow(deprecated)]
        {
            get_sysvar(
                invoke_context.get_sysvar_cache().get_fees(),
                var_addr,
                invoke_context.get_check_aligned(),
                memory_mapping,
                invoke_context,
            )
        }
    }
);

declare_builtin_function!(
    /// Get a Rent sysvar
    SyscallGetRentSysvar,
    fn rust(
        invoke_context: &mut InvokeContext,
        var_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        get_sysvar(
            invoke_context.get_sysvar_cache().get_rent(),
            var_addr,
            invoke_context.get_check_aligned(),
            memory_mapping,
            invoke_context,
        )
    }
);

declare_builtin_function!(
    /// Get a Last Restart Slot sysvar
    SyscallGetLastRestartSlotSysvar,
    fn rust(
        invoke_context: &mut InvokeContext,
        var_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        get_sysvar(
            invoke_context.get_sysvar_cache().get_last_restart_slot(),
            var_addr,
            invoke_context.get_check_aligned(),
            memory_mapping,
            invoke_context,
        )
    }
);
