/// Generic Path-Dependent Optimality Engine
///
/// Generic over discrete event types `e ∈ E` with:
/// - Finite discrete outcome sets
/// - Deterministic state transitions: `R_w = R_{w-1} ⊕ e_w`
/// - Additive utility functions
///
/// Team records (Win/Loss/Tie) are one implementation of this generic framework.
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;

/// Entity identifier (team, player, etc.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId(pub u8);

/// Generic trait for discrete event outcomes
///
/// Any type implementing this can be used in the MDP engine
pub trait EventOutcome: Clone + Copy + PartialEq + Eq + Hash + Debug {
	/// Convert outcome to numeric score for utility calculations
	fn score(&self) -> f64;

	/// Optional: semantic description
	fn description(&self) -> &'static str {
		"outcome"
	}
}

/// Generic trait for cumulative state records
///
/// Represents R_w(t) - the accumulated state for entity t at week w
pub trait CumulativeRecord: Clone + PartialEq + Eq + Hash + Debug + Default {
	type Outcome: EventOutcome;

	/// Apply a weekly outcome to update the cumulative record
	/// Implements the ⊕ operator component for a single entity
	fn apply_outcome(&mut self, outcome: Self::Outcome);

	/// Compute derived metrics if needed (e.g., win percentage)
	#[must_use]
	fn derived_metric(&self) -> f64 {
		0.0
	}
}

/// Generic state R_w: cumulative records for all entities at end of period w
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State<R: CumulativeRecord> {
	records: HashMap<EntityId, R>,
}

impl<R: CumulativeRecord> Hash for State<R> {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		// Convert HashMap to sorted Vec for deterministic hashing
		let mut items: Vec<_> = self.records.iter().collect();
		items.sort_by_key(|(k, _)| *k);
		for (k, v) in items {
			k.hash(state);
			v.hash(state);
		}
	}
}

impl<R: CumulativeRecord> State<R> {
	#[must_use]
	pub fn new() -> Self {
		Self { records: HashMap::new() }
	}

	pub fn get_record(&self, entity: EntityId) -> R {
		self.records.get(&entity).cloned().unwrap_or_default()
	}

	pub fn set_record(&mut self, entity: EntityId, record: R) {
		self.records.insert(entity, record);
	}

	/// Transition function: R_w = R_{w-1} ⊕ e_w
	/// Apply period outcomes to produce new state
	pub fn apply_period(&self, period_outcomes: &PeriodOutcomes<R::Outcome>) -> Self {
		let mut new_state = self.clone();
		for (&entity, &outcome) in &period_outcomes.outcomes {
			let mut record = new_state.get_record(entity);
			record.apply_outcome(outcome);
			new_state.set_record(entity, record);
		}
		new_state
	}
}

impl<R: CumulativeRecord> Default for State<R> {
	fn default() -> Self {
		Self::new()
	}
}

/// Period outcomes for all entities: e_w
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodOutcomes<O: EventOutcome> {
	outcomes: HashMap<EntityId, O>,
}

impl<O: EventOutcome> Hash for PeriodOutcomes<O> {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		// Convert HashMap to sorted Vec for deterministic hashing
		let mut items: Vec<_> = self.outcomes.iter().collect();
		items.sort_by_key(|(k, _)| *k);
		for (k, v) in items {
			k.hash(state);
			v.hash(state);
		}
	}
}

impl<O: EventOutcome> PeriodOutcomes<O> {
	pub fn new() -> Self {
		Self { outcomes: HashMap::new() }
	}

	pub fn set_outcome(&mut self, entity: EntityId, outcome: O) {
		self.outcomes.insert(entity, outcome);
	}

	pub fn get_outcome(&self, entity: EntityId) -> Option<O> {
		self.outcomes.get(&entity).copied()
	}

	pub fn get_score(&self, entity: EntityId) -> f64 {
		self.get_outcome(entity).map_or(0.0, |o| o.score())
	}
}

impl<O: EventOutcome> Default for PeriodOutcomes<O> {
	fn default() -> Self {
		Self::new()
	}
}

/// Hierarchical weights for entity importance
#[derive(Debug, Clone, Copy)]
pub struct HierarchicalWeights {
	/// Weight for primary entity's own outcome
	pub w_primary: f64,
	/// Weight for tier-1 rivals (e.g., divisional)
	pub w_tier1: f64,
	/// Weight for tier-2 rivals (e.g., conference)
	pub w_tier2: f64,
	/// Weight for tier-3 rivals (e.g., other-conference)
	pub w_tier3: f64,
}

impl Default for HierarchicalWeights {
	fn default() -> Self {
		Self {
			w_primary: 1.0,
			w_tier1: 0.6,
			w_tier2: 0.3,
			w_tier3: 0.1,
		}
	}
}

impl HierarchicalWeights {
	/// Validate that weights satisfy constraints
	///
	/// # Errors
	///
	/// Returns an error if:
	/// - `w_primary` is not positive
	/// - Tier weights are not properly ordered (w_tier1 > w_tier2 > w_tier3)
	/// - `w_tier3` is negative
	pub fn validate(&self) -> Result<(), String> {
		if self.w_primary <= 0.0 {
			return Err("w_primary must be positive".to_string());
		}
		if self.w_tier1 <= self.w_tier2 {
			return Err("w_tier1 must be greater than w_tier2".to_string());
		}
		if self.w_tier2 <= self.w_tier3 {
			return Err("w_tier2 must be greater than w_tier3".to_string());
		}
		if self.w_tier3 < 0.0 {
			return Err("w_tier3 must be non-negative".to_string());
		}
		Ok(())
	}
}

/// Generic hierarchical structure
#[derive(Debug, Clone)]
pub struct EntityHierarchy {
	/// Primary entity (e.g., favorite team)
	pub primary: EntityId,
	/// Tier-1 rivals
	pub tier1_rivals: Vec<EntityId>,
	/// Tier-2 rivals
	pub tier2_rivals: Vec<EntityId>,
	/// Tier-3 rivals
	pub tier3_rivals: Vec<EntityId>,
}

impl EntityHierarchy {
	pub fn all_entities(&self) -> Vec<EntityId> {
		let mut entities = vec![self.primary];
		entities.extend(&self.tier1_rivals);
		entities.extend(&self.tier2_rivals);
		entities.extend(&self.tier3_rivals);
		entities
	}
}

/// Value function cache for dynamic programming
type ValueCache<R> = HashMap<(usize, State<R>), f64>;

/// Generic path-dependent optimality calculator
///
/// Works with any event type implementing EventOutcome and CumulativeRecord
pub struct GenericOptimalityEngine<R: CumulativeRecord> {
	hierarchy: EntityHierarchy,
	weights: HierarchicalWeights,
	pub value_cache: ValueCache<R>,
	max_periods: usize,
}

impl<R: CumulativeRecord> GenericOptimalityEngine<R> {
	pub fn new(hierarchy: EntityHierarchy, weights: HierarchicalWeights, max_periods: usize) -> Result<Self, String> {
		weights.validate()?;
		Ok(Self {
			hierarchy,
			weights,
			value_cache: HashMap::new(),
			max_periods,
		})
	}

	/// Utility function U(R_w, e_w): immediate reward for period w
	pub fn period_utility(&self, _state: &State<R>, period_outcomes: &PeriodOutcomes<R::Outcome>) -> f64 {
		let primary_score = period_outcomes.get_score(self.hierarchy.primary);

		// Primary entity contribution
		let mut utility = self.weights.w_primary * primary_score;

		// Tier-1 rivals contribution
		for &rival in &self.hierarchy.tier1_rivals {
			let rival_score = period_outcomes.get_score(rival);
			let diff = (primary_score - rival_score).max(0.0);
			utility += self.weights.w_tier1 * diff;
		}

		// Tier-2 rivals contribution
		for &rival in &self.hierarchy.tier2_rivals {
			let rival_score = period_outcomes.get_score(rival);
			let diff = (primary_score - rival_score).max(0.0);
			utility += self.weights.w_tier2 * diff;
		}

		// Tier-3 rivals contribution
		for &rival in &self.hierarchy.tier3_rivals {
			let rival_score = period_outcomes.get_score(rival);
			let diff = (primary_score - rival_score).max(0.0);
			utility += self.weights.w_tier3 * diff;
		}

		utility
	}

	/// Maximum possible utility for a single period
	pub fn max_period_utility(&self) -> f64 {
		self.weights.w_primary.mul_add(
			1.0,
			f64::from(self.hierarchy.tier1_rivals.len() as u32) * self.weights.w_tier1
				+ f64::from(self.hierarchy.tier2_rivals.len() as u32) * self.weights.w_tier2
				+ f64::from(self.hierarchy.tier3_rivals.len() as u32) * self.weights.w_tier3,
		)
	}

	/// Value function V_w(R_{w-1}): max achievable cumulative utility from period w onward
	pub fn value_function(&mut self, period: usize, state: &State<R>, feasible_outcomes: &[PeriodOutcomes<R::Outcome>]) -> f64 {
		// Terminal condition
		if period > self.max_periods {
			return 0.0;
		}

		// Check cache
		let cache_key = (period, state.clone());
		if let Some(&cached_value) = self.value_cache.get(&cache_key) {
			return cached_value;
		}

		// Compute max over all feasible outcomes
		let mut max_value: f64 = 0.0;
		for outcome in feasible_outcomes {
			let immediate_utility = self.period_utility(state, outcome);
			let next_state = state.apply_period(outcome);
			let future_value = self.value_function(period + 1, &next_state, feasible_outcomes);
			let total_value: f64 = immediate_utility + future_value;
			max_value = max_value.max(total_value);
		}

		// Cache and return
		self.value_cache.insert(cache_key, max_value);
		max_value
	}

	/// Compute optimal outcome e*_w for a given state
	pub fn optimal_outcome(&mut self, period: usize, state: &State<R>, feasible_outcomes: &[PeriodOutcomes<R::Outcome>]) -> Option<PeriodOutcomes<R::Outcome>> {
		if period > self.max_periods || feasible_outcomes.is_empty() {
			return None;
		}

		let mut best_outcome = None;
		let mut best_value = f64::NEG_INFINITY;

		for outcome in feasible_outcomes {
			let immediate_utility = self.period_utility(state, outcome);
			let next_state = state.apply_period(outcome);
			let future_value = self.value_function(period + 1, &next_state, feasible_outcomes);
			let total_value = immediate_utility + future_value;

			if total_value > best_value {
				best_value = total_value;
				best_outcome = Some(outcome.clone());
			}
		}

		best_outcome
	}

	/// Observed cumulative utility V^obs_w(R_{w-1})
	pub fn observed_value(&mut self, period: usize, state: &State<R>, observed_outcome: &PeriodOutcomes<R::Outcome>, feasible_outcomes: &[PeriodOutcomes<R::Outcome>]) -> f64 {
		if period > self.max_periods {
			return 0.0;
		}

		let immediate_utility = self.period_utility(state, observed_outcome);
		let next_state = state.apply_period(observed_outcome);
		let future_value = self.value_function(period + 1, &next_state, feasible_outcomes);
		immediate_utility + future_value
	}

	/// Per-period optimality score: Optimality_w ∈ [0, 1]
	pub fn period_optimality(
		&mut self,
		period: usize,
		state: &State<R>,
		observed_outcome: &PeriodOutcomes<R::Outcome>,
		feasible_outcomes: &[PeriodOutcomes<R::Outcome>],
	) -> f64 {
		let observed_val = self.observed_value(period, state, observed_outcome, feasible_outcomes);
		let optimal_val = self.value_function(period, state, feasible_outcomes);

		if optimal_val > 0.0 {
			(observed_val / optimal_val).clamp(0.0, 1.0)
		} else {
			0.0
		}
	}

	/// Season-level optimality: average across all periods
	pub fn season_optimality(&mut self, observed_periods: &[(State<R>, PeriodOutcomes<R::Outcome>)], feasible_outcomes: &[PeriodOutcomes<R::Outcome>]) -> f64 {
		if observed_periods.is_empty() {
			return 0.0;
		}

		let mut total = 0.0;
		for (period_idx, (state, outcome)) in observed_periods.iter().enumerate() {
			let period_num = period_idx + 1;
			let opt = self.period_optimality(period_num, state, outcome, feasible_outcomes);
			total += opt;
		}

		total / (observed_periods.len() as f64)
	}

	pub fn clear_cache(&mut self) {
		self.value_cache.clear();
	}
}

// ============================================================================
// CONCRETE IMPLEMENTATION: Team Game Outcomes (Win/Loss/Tie)
// ============================================================================

/// Game outcome for team sports
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameOutcome {
	Loss,
	Tie,
	Win,
}

impl EventOutcome for GameOutcome {
	fn score(&self) -> f64 {
		match self {
			GameOutcome::Loss => 0.0,
			GameOutcome::Tie => 0.5,
			GameOutcome::Win => 1.0,
		}
	}

	fn description(&self) -> &'static str {
		match self {
			GameOutcome::Loss => "loss",
			GameOutcome::Tie => "tie",
			GameOutcome::Win => "win",
		}
	}
}

/// Team record: R_w(t) = (wins, losses, ties)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TeamRecord {
	pub wins: u8,
	pub losses: u8,
	pub ties: u8,
}

impl CumulativeRecord for TeamRecord {
	type Outcome = GameOutcome;

	fn apply_outcome(&mut self, outcome: Self::Outcome) {
		match outcome {
			GameOutcome::Win => self.wins += 1,
			GameOutcome::Loss => self.losses += 1,
			GameOutcome::Tie => self.ties += 1,
		}
	}

	fn derived_metric(&self) -> f64 {
		let total = self.wins + self.losses + self.ties;
		if total == 0 {
			return 0.0;
		}
		(f64::from(self.wins) + 0.5 * f64::from(self.ties)) / f64::from(total)
	}
}

impl Default for TeamRecord {
	fn default() -> Self {
		Self { wins: 0, losses: 0, ties: 0 }
	}
}

/// Convenience type alias for team-based engine
pub type TeamOptimalityEngine = GenericOptimalityEngine<TeamRecord>;

// ============================================================================
// EXAMPLE: Alternative Event Type - Turnover Tracking
// ============================================================================

/// Turnover outcome per game
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TurnoverOutcome {
	Zero,
	One,
	Two,
	ThreePlus,
}

impl EventOutcome for TurnoverOutcome {
	fn score(&self) -> f64 {
		match self {
			TurnoverOutcome::Zero => 1.0, // Best
			TurnoverOutcome::One => 0.66,
			TurnoverOutcome::Two => 0.33,
			TurnoverOutcome::ThreePlus => 0.0, // Worst
		}
	}

	fn description(&self) -> &'static str {
		match self {
			TurnoverOutcome::Zero => "0 turnovers",
			TurnoverOutcome::One => "1 turnover",
			TurnoverOutcome::Two => "2 turnovers",
			TurnoverOutcome::ThreePlus => "3+ turnovers",
		}
	}
}

/// Cumulative turnover record
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TurnoverRecord {
	pub total_turnovers: u16,
	pub games_played: u8,
}

impl CumulativeRecord for TurnoverRecord {
	type Outcome = TurnoverOutcome;

	fn apply_outcome(&mut self, outcome: Self::Outcome) {
		self.games_played += 1;
		self.total_turnovers += match outcome {
			TurnoverOutcome::Zero => 0,
			TurnoverOutcome::One => 1,
			TurnoverOutcome::Two => 2,
			TurnoverOutcome::ThreePlus => 3,
		};
	}

	fn derived_metric(&self) -> f64 {
		if self.games_played == 0 {
			return 0.0;
		}
		f64::from(self.total_turnovers) / f64::from(self.games_played)
	}
}

impl Default for TurnoverRecord {
	fn default() -> Self {
		Self {
			total_turnovers: 0,
			games_played: 0,
		}
	}
}

/// Convenience type alias for turnover-based engine
pub type TurnoverOptimalityEngine = GenericOptimalityEngine<TurnoverRecord>;
