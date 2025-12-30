use super::error::Result;
use super::{OrchestratorCommandData, TimeMs};
use tokio::sync::oneshot;

/// Internal command type used inside the orchestrator engine
#[derive(Debug)]
pub enum OrchestratorCommand {
	// FSM commands with response
	Configure {
		config: OrchestratorCommandData,
		response: oneshot::Sender<Result<()>>,
	},
	Start {
		response: oneshot::Sender<Result<()>>,
	},
	Pause {
		response: oneshot::Sender<Result<()>>,
	},
	Resume {
		response: oneshot::Sender<Result<()>>,
	},
	Stop {
		response: oneshot::Sender<Result<()>>,
	},
	Reset {
		response: oneshot::Sender<Result<()>>,
	},

	// Fire-and-forget
	ForceScene(String),
	SkipCurrentScene,
	UpdateStreamStatus {
		is_streaming: bool,
		stream_time: TimeMs,
		timecode: String,
	},
}

impl OrchestratorCommand {
	/// Convert from transport/entity command + optional response channel
	pub fn from_data(cmd: OrchestratorCommandData, response: Option<oneshot::Sender<Result<()>>>) -> Self {
		match cmd {
			OrchestratorCommandData::Configure(config) => OrchestratorCommand::Configure {
				config: OrchestratorCommandData::Configure(config),
				response: response.expect("FSM command requires response channel"),
			},
			OrchestratorCommandData::Start => OrchestratorCommand::Start {
				response: response.expect("FSM command requires response channel"),
			},
			OrchestratorCommandData::Pause => OrchestratorCommand::Pause {
				response: response.expect("FSM command requires response channel"),
			},
			OrchestratorCommandData::Resume => OrchestratorCommand::Resume {
				response: response.expect("FSM command requires response channel"),
			},
			OrchestratorCommandData::Stop => OrchestratorCommand::Stop {
				response: response.expect("FSM command requires response channel"),
			},
			OrchestratorCommandData::Reset => OrchestratorCommand::Reset {
				response: response.expect("FSM command requires response channel"),
			},
			OrchestratorCommandData::ForceScene(scene) => OrchestratorCommand::ForceScene(scene),
			OrchestratorCommandData::SkipCurrentScene => OrchestratorCommand::SkipCurrentScene,
			OrchestratorCommandData::UpdateStreamStatus {
				is_streaming,
				stream_time,
				timecode,
			} => OrchestratorCommand::UpdateStreamStatus {
				is_streaming,
				stream_time,
				timecode,
			},
		}
	}
}
