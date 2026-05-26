use super::super::service::{
    ServiceArgs, ServiceCommand, ServiceCommandRequestInput, build_service_command_request,
};
use crate::{
    commands::input::{AppCommand, ServiceCommandInput},
    service::ServiceAction,
};

pub(super) fn parse_service_command(args: ServiceArgs) -> anyhow::Result<AppCommand> {
    match args.command {
        ServiceCommand::Install(args) => Ok(AppCommand::Service(ServiceCommandInput {
            request: build_service_command_request(ServiceCommandRequestInput {
                action: ServiceAction::Install,
                manager: args.manager,
                mode: args.mode,
                dry_run: args.dry_run,
                unit_dir: args.unit_dir,
                config_dir: args.config_dir,
                state_dir: args.state_dir,
                log_dir: args.log_dir,
                binary: args.binary,
            })?,
            json: args.json,
        })),
        ServiceCommand::Uninstall(args) => Ok(AppCommand::Service(ServiceCommandInput {
            request: build_service_command_request(ServiceCommandRequestInput {
                action: ServiceAction::Uninstall,
                manager: args.manager,
                mode: args.mode,
                dry_run: args.dry_run,
                unit_dir: args.unit_dir,
                config_dir: args.config_dir,
                state_dir: args.state_dir,
                log_dir: args.log_dir,
                binary: args.binary,
            })?,
            json: args.json,
        })),
        ServiceCommand::Doctor(args) => Ok(AppCommand::Service(ServiceCommandInput {
            request: build_service_command_request(ServiceCommandRequestInput {
                action: ServiceAction::Doctor,
                manager: args.manager,
                mode: args.mode,
                dry_run: true,
                unit_dir: args.unit_dir,
                config_dir: args.config_dir,
                state_dir: args.state_dir,
                log_dir: args.log_dir,
                binary: args.binary,
            })?,
            json: args.json,
        })),
    }
}
