impl KeepaliveManager {
	pub fn handle_ping_failure(&mut self) {
		self.consecutive_failures += 1;
		self.state = KeepaliveState::Failed { last_failure: Instant::now() };
	}
}
