use {
    crate::{
        admin_rpc_service,
        commands::{FromClapArgMatches, Result},
    },
    clap::{App, Arg, ArgMatches, SubCommand, value_t},
    std::path::Path,
};

const COMMAND: &str = "set-execution-time-limit";

#[derive(Debug, PartialEq)]
pub struct SetExecutionTimeLimitArgs {
    pub limit_ms: u64,
}

impl FromClapArgMatches for SetExecutionTimeLimitArgs {
    fn from_clap_arg_match(matches: &ArgMatches) -> Result<Self> {
        Ok(SetExecutionTimeLimitArgs {
            limit_ms: value_t!(matches, "limit_ms", u64)?,
        })
    }
}

pub fn command<'a>() -> App<'a, 'a> {
    SubCommand::with_name(COMMAND)
        .about("Adjust the leader's transaction execution time limit")
        .arg(
            Arg::with_name("limit_ms")
                .value_name("MILLISECONDS")
                .takes_value(true)
                .index(1)
                .help("New execution time limit in milliseconds, or 0 to disable it"),
        )
        .after_help("Note: the new limit only applies to the currently running validator instance")
}

pub fn execute(matches: &ArgMatches, ledger_path: &Path) -> Result<()> {
    let set_execution_time_limit_args = SetExecutionTimeLimitArgs::from_clap_arg_match(matches)?;

    let admin_client = admin_rpc_service::connect(ledger_path);
    admin_rpc_service::runtime().block_on(async move {
        admin_client
            .await?
            .set_execution_time_limit(set_execution_time_limit_args.limit_ms)
            .await
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use {super::*, crate::commands::tests::verify_args_struct_by_command};

    #[test]
    fn verify_args_struct_by_command_set_execution_time_limit_default() {
        let matches = command().get_matches_from(vec![COMMAND]);
        assert!(SetExecutionTimeLimitArgs::from_clap_arg_match(&matches).is_err());
    }

    #[test]
    fn verify_args_struct_by_command_set_execution_time_limit_with_limit() {
        verify_args_struct_by_command(
            command(),
            vec![COMMAND, "10"],
            SetExecutionTimeLimitArgs { limit_ms: 10 },
        );
    }
}
