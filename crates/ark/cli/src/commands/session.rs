use anyhow::{anyhow, Result};
use clap::{ArgAction, Parser, Subcommand};
use kube::Client;
use tracing::{info, instrument, Level};
use vine_api::{user::UserSpec, user_auth::UserSessionResponse};

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum Command {
    Batch(BatchArgs),
    Login(LoginArgs),
    Logout(LogoutArgs),
}

impl Command {
    #[instrument(level = Level::INFO, skip_all, err(Display))]
    pub(crate) async fn run(self) -> Result<()> {
        fn validate_session_response(response: UserSessionResponse) -> Result<()> {
            match response {
                UserSessionResponse::Accept {
                    box_quota: _,
                    user:
                        UserSpec {
                            name,
                            contact: _,
                            detail: _,
                        },
                } => {
                    info!("Ok ({name})");
                    Ok(())
                }
                UserSessionResponse::Error(error) => Err(error.into()),
            }
        }

        let kube = Client::try_default()
            .await
            .map_err(|error| anyhow!("failed to load kubernetes config: {error}"))?;

        match self {
            Self::Batch(command) => command
                .run(kube)
                .await
                .map_err(|error| anyhow!("failed to command: {error}")),
            Self::Login(command) => command
                .run(kube)
                .await
                .map_err(|error| anyhow!("failed to login: {error}"))
                .and_then(validate_session_response),
            Self::Logout(command) => command
                .run(kube)
                .await
                .map_err(|error| anyhow!("failed to logout: {error}"))
                .and_then(validate_session_response),
        }
    }
}

#[derive(Clone, Debug, Parser)]
pub(crate) struct BatchArgs {
    #[arg(long, default_value_t = false)]
    detach: bool,

    #[arg(action = ArgAction::Append, value_name = "COMMAND")]
    shell: Vec<String>,

    #[arg(short, long, default_value_t = false)]
    terminal: bool,

    #[arg(short, long, env = "VINE_SESSION_USER", value_name = "PATTERN")]
    user_pattern: Option<String>,
}

impl BatchArgs {
    #[instrument(level = Level::INFO, skip_all, err(Display))]
    pub(crate) async fn run(self, kube: Client) -> Result<()> {
        let Self {
            detach,
            shell,
            terminal,
            user_pattern,
        } = self;

        let mut command = vec![];

        if terminal {
            command.push("xfce4-terminal".into());
            command.push("--disable-server".into());
            command.push("-x".into());
        }

        command.push("/usr/bin/env".into());
        command.push("sh".into());
        command.push("-c".into());
        command.push(shell.join(" "));

        let num_boxes = ::vine_session::BatchCommandArgs {
            command,
            users: match user_pattern.as_ref() {
                Some(re) => ::vine_session::BatchCommandUsers::Pattern(re),
                None => ::vine_session::BatchCommandUsers::All,
            },
            wait: !detach,
        }
        .exec(&kube)
        .await?;

        info!("Executed in {num_boxes} boxes.");
        Ok(())
    }
}

#[derive(Clone, Debug, Parser)]
pub(crate) struct LoginArgs {
    #[arg(long, env = "VINE_SESSION_BOX", value_name = "NAME")]
    r#box: String,

    #[arg(long, env = "VINE_SESSION_USER", value_name = "NAME")]
    user: String,

    #[arg(long, env = "VINE_SESSION_LOGOUT_ON_FAILED")]
    logout_on_failed: bool,
}

impl LoginArgs {
    #[instrument(level = Level::INFO, skip_all, err(Display))]
    pub(crate) async fn run(self, kube: Client) -> Result<UserSessionResponse> {
        let Self {
            r#box: box_name,
            user: user_name,
            logout_on_failed,
        } = self;

        ::vine_rbac::login::execute(&kube, &box_name, &user_name, logout_on_failed).await
    }
}

#[derive(Clone, Debug, Parser)]
pub(crate) struct LogoutArgs {
    #[arg(long, env = "VINE_SESSION_BOX", value_name = "NAME")]
    r#box: String,

    #[arg(long, env = "VINE_SESSION_USER", value_name = "NAME")]
    user: String,
}

impl LogoutArgs {
    #[instrument(level = Level::INFO, skip_all, err(Display))]
    pub(crate) async fn run(self, kube: Client) -> Result<UserSessionResponse> {
        let Self {
            r#box: box_name,
            user: user_name,
        } = &self;

        ::vine_rbac::logout::execute(&kube, box_name, user_name).await
    }
}
