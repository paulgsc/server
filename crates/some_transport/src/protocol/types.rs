#[derive(Debug, Clone, Copy)]
pub enum CloseCode {
	Normal = 1000,
	GoingAway = 1001,
	ProtocolError = 1002,
	Unsupported = 1003,
	NoStatusRcvd = 1005,
	Abnormal = 1006,
	InvalidFramePayloadData = 1007,
	PolicyViolation = 1008,
	MessageTooBig = 1009,
	MandatoryExtension = 1010,
	InternalServerError = 1011,
	ServiceRestart = 1012,
	TryAgainLater = 1013,
	BadGateway = 1014,
	TlsHandshake = 1015,
}

#[derive(Debug, Clone)]
pub enum CloseInitiator {
	Local,
	Remote,
}
#[derive(Debug, Clone)]
pub enum ContentEncoding {
	None,
	Gzip,
	Deflate,
}

#[derive(Debug, Clone)]
pub enum DeliveryMode {
	BestEffort,
	AtLeastOnce,
	ExactlyOnce,
}
