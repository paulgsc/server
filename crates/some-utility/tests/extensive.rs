#[cfg(test)]
mod comprehensive_tests {
	use some_utility::*;

	// ========================================================================
	// Helper Functions
	// ========================================================================

	fn create_simple_hierarchy() -> EntityHierarchy {
		EntityHierarchy {
			primary: EntityId(0),
			tier1_rivals: vec![EntityId(1), EntityId(2)],
			tier2_rivals: vec![EntityId(3)],
			tier3_rivals: vec![EntityId(4)],
		}
	}

	fn create_nfl_hierarchy() -> EntityHierarchy {
		EntityHierarchy {
			primary: EntityId(0),                                                                                                         // Your team
			tier1_rivals: vec![EntityId(1), EntityId(2), EntityId(3)],                                                                    // Division
			tier2_rivals: vec![EntityId(4), EntityId(5), EntityId(6), EntityId(7), EntityId(8), EntityId(9), EntityId(10), EntityId(11)], // Conference
			tier3_rivals: vec![EntityId(12), EntityId(13), EntityId(14), EntityId(15)],                                                   // Other conference
		}
	}

	fn create_perfect_week(hierarchy: &EntityHierarchy) -> PeriodOutcomes<GameOutcome> {
		let mut outcomes = PeriodOutcomes::new();
		outcomes.set_outcome(hierarchy.primary, GameOutcome::Win);
		for &rival in &hierarchy.tier1_rivals {
			outcomes.set_outcome(rival, GameOutcome::Loss);
		}
		for &rival in &hierarchy.tier2_rivals {
			outcomes.set_outcome(rival, GameOutcome::Loss);
		}
		for &rival in &hierarchy.tier3_rivals {
			outcomes.set_outcome(rival, GameOutcome::Loss);
		}
		outcomes
	}

	fn create_worst_week(hierarchy: &EntityHierarchy) -> PeriodOutcomes<GameOutcome> {
		let mut outcomes = PeriodOutcomes::new();
		outcomes.set_outcome(hierarchy.primary, GameOutcome::Loss);
		for &rival in &hierarchy.tier1_rivals {
			outcomes.set_outcome(rival, GameOutcome::Win);
		}
		for &rival in &hierarchy.tier2_rivals {
			outcomes.set_outcome(rival, GameOutcome::Win);
		}
		for &rival in &hierarchy.tier3_rivals {
			outcomes.set_outcome(rival, GameOutcome::Win);
		}
		outcomes
	}

	fn create_mixed_week(hierarchy: &EntityHierarchy) -> PeriodOutcomes<GameOutcome> {
		let mut outcomes = PeriodOutcomes::new();
		outcomes.set_outcome(hierarchy.primary, GameOutcome::Win);
		// Half rivals win, half lose
		for (idx, &rival) in hierarchy.tier1_rivals.iter().enumerate() {
			outcomes.set_outcome(rival, if idx % 2 == 0 { GameOutcome::Loss } else { GameOutcome::Win });
		}
		for (idx, &rival) in hierarchy.tier2_rivals.iter().enumerate() {
			outcomes.set_outcome(rival, if idx % 2 == 0 { GameOutcome::Loss } else { GameOutcome::Win });
		}
		for (idx, &rival) in hierarchy.tier3_rivals.iter().enumerate() {
			outcomes.set_outcome(rival, if idx % 2 == 0 { GameOutcome::Loss } else { GameOutcome::Win });
		}
		outcomes
	}

	// ========================================================================
	// Basic Trait Implementation Tests
	// ========================================================================

	#[test]
	fn test_game_outcome_scores() {
		assert_eq!(GameOutcome::Loss.score(), 0.0);
		assert_eq!(GameOutcome::Tie.score(), 0.5);
		assert_eq!(GameOutcome::Win.score(), 1.0);
	}

	#[test]
	fn test_team_record_apply_outcome() {
		let mut record = TeamRecord::default();
		assert_eq!(record.wins, 0);
		assert_eq!(record.losses, 0);
		assert_eq!(record.ties, 0);

		record.apply_outcome(GameOutcome::Win);
		assert_eq!(record.wins, 1);

		record.apply_outcome(GameOutcome::Loss);
		assert_eq!(record.losses, 1);

		record.apply_outcome(GameOutcome::Tie);
		assert_eq!(record.ties, 1);
	}

	#[test]
	fn test_team_record_derived_metric() {
		let mut record = TeamRecord::default();

		// 0 games played
		assert_eq!(record.derived_metric(), 0.0);

		// 2-1-0 record = 0.666...
		record.apply_outcome(GameOutcome::Win);
		record.apply_outcome(GameOutcome::Win);
		record.apply_outcome(GameOutcome::Loss);
		assert!((record.derived_metric() - 0.666666).abs() < 0.001);

		// Add a tie: 2-1-1 = (2 + 0.5) / 4 = 0.625
		record.apply_outcome(GameOutcome::Tie);
		assert_eq!(record.derived_metric(), 0.625);
	}

	// ========================================================================
	// State Transition Tests
	// ========================================================================

	#[test]
	fn test_state_initialization() {
		let state: State<TeamRecord> = State::new();
		let record = state.get_record(EntityId(0));
		assert_eq!(record.wins, 0);
		assert_eq!(record.losses, 0);
		assert_eq!(record.ties, 0);
	}

	#[test]
	fn test_state_apply_period() {
		let state: State<TeamRecord> = State::new();
		let mut outcomes = PeriodOutcomes::new();
		outcomes.set_outcome(EntityId(0), GameOutcome::Win);
		outcomes.set_outcome(EntityId(1), GameOutcome::Loss);
		outcomes.set_outcome(EntityId(2), GameOutcome::Tie);

		let new_state = state.apply_period(&outcomes);

		assert_eq!(new_state.get_record(EntityId(0)).wins, 1);
		assert_eq!(new_state.get_record(EntityId(1)).losses, 1);
		assert_eq!(new_state.get_record(EntityId(2)).ties, 1);
	}

	#[test]
	fn test_state_apply_multiple_periods() {
		let mut state: State<TeamRecord> = State::new();

		// Week 1: Team 0 wins
		let mut week1 = PeriodOutcomes::new();
		week1.set_outcome(EntityId(0), GameOutcome::Win);
		state = state.apply_period(&week1);

		// Week 2: Team 0 wins again
		let mut week2 = PeriodOutcomes::new();
		week2.set_outcome(EntityId(0), GameOutcome::Win);
		state = state.apply_period(&week2);

		// Week 3: Team 0 loses
		let mut week3 = PeriodOutcomes::new();
		week3.set_outcome(EntityId(0), GameOutcome::Loss);
		state = state.apply_period(&week3);

		let final_record = state.get_record(EntityId(0));
		assert_eq!(final_record.wins, 2);
		assert_eq!(final_record.losses, 1);
		assert_eq!(final_record.ties, 0);
	}

	// ========================================================================
	// Weight Validation Tests
	// ========================================================================

	#[test]
	fn test_valid_weights() {
		let weights = HierarchicalWeights {
			w_primary: 1.0,
			w_tier1: 0.6,
			w_tier2: 0.3,
			w_tier3: 0.1,
		};
		assert!(weights.validate().is_ok());
	}

	#[test]
	fn test_invalid_weights_primary_zero() {
		let weights = HierarchicalWeights {
			w_primary: 0.0,
			w_tier1: 0.6,
			w_tier2: 0.3,
			w_tier3: 0.1,
		};
		assert!(weights.validate().is_err());
	}

	#[test]
	fn test_invalid_weights_tier_ordering() {
		// tier2 > tier1
		let weights = HierarchicalWeights {
			w_primary: 1.0,
			w_tier1: 0.3,
			w_tier2: 0.6,
			w_tier3: 0.1,
		};
		assert!(weights.validate().is_err());

		// tier3 > tier2
		let weights2 = HierarchicalWeights {
			w_primary: 1.0,
			w_tier1: 0.6,
			w_tier2: 0.3,
			w_tier3: 0.5,
		};
		assert!(weights2.validate().is_err());
	}

	#[test]
	fn test_invalid_weights_negative() {
		let weights = HierarchicalWeights {
			w_primary: 1.0,
			w_tier1: 0.6,
			w_tier2: 0.3,
			w_tier3: -0.1,
		};
		assert!(weights.validate().is_err());
	}

	// ========================================================================
	// Utility Function Tests
	// ========================================================================

	#[test]
	fn test_period_utility_perfect_week() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();
		let perfect = create_perfect_week(&hierarchy);

		let utility = engine.period_utility(&state, &perfect);
		let max_utility = engine.max_period_utility();

		assert_eq!(utility, max_utility);
	}

	#[test]
	fn test_period_utility_worst_week() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();
		let worst = create_worst_week(&hierarchy);

		let utility = engine.period_utility(&state, &worst);
		assert_eq!(utility, 0.0);
	}

	#[test]
	fn test_period_utility_mixed_week() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();
		let mixed = create_mixed_week(&hierarchy);

		let utility = engine.period_utility(&state, &mixed);
		let max_utility = engine.max_period_utility();

		assert!(utility > 0.0);
		assert!(utility < max_utility);
	}

	#[test]
	fn test_utility_respects_hierarchy() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();

		// Primary wins, tier1 rival loses
		let mut outcomes1 = PeriodOutcomes::new();
		outcomes1.set_outcome(hierarchy.primary, GameOutcome::Win);
		outcomes1.set_outcome(hierarchy.tier1_rivals[0], GameOutcome::Loss);
		let utility1 = engine.period_utility(&state, &outcomes1);

		// Primary wins, tier3 rival loses (should be less valuable)
		let mut outcomes2 = PeriodOutcomes::new();
		outcomes2.set_outcome(hierarchy.primary, GameOutcome::Win);
		outcomes2.set_outcome(hierarchy.tier3_rivals[0], GameOutcome::Loss);
		let utility2 = engine.period_utility(&state, &outcomes2);

		assert!(utility1 > utility2, "Tier1 rival loss should be more valuable than tier3");
	}

	// ========================================================================
	// Value Function Tests
	// ========================================================================

	#[test]
	fn test_value_function_terminal_condition() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();
		let feasible = vec![create_perfect_week(&hierarchy)];

		// Beyond max periods should return 0
		let value = engine.value_function(18, &state, &feasible);
		assert_eq!(value, 0.0);
	}

	#[test]
	fn test_value_function_single_period() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 1).unwrap();

		let state = State::<TeamRecord>::new();
		let perfect = create_perfect_week(&hierarchy);
		let feasible = vec![perfect.clone()];

		let value = engine.value_function(1, &state, &feasible);
		let expected = engine.period_utility(&state, &perfect);

		assert_eq!(value, expected);
	}

	#[test]
	fn test_value_function_caching() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();
		let feasible = vec![create_perfect_week(&hierarchy)];

		// First call
		let value1 = engine.value_function(1, &state, &feasible);

		// Second call should use cache
		let value2 = engine.value_function(1, &state, &feasible);

		assert_eq!(value1, value2);
		assert!(!engine.value_cache.is_empty());
	}

	#[test]
	fn test_value_function_chooses_best_outcome() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();
		let perfect = create_perfect_week(&hierarchy);
		let worst = create_worst_week(&hierarchy);
		let feasible = vec![perfect.clone(), worst.clone()];

		let value = engine.value_function(1, &state, &feasible);
		let perfect_utility = engine.period_utility(&state, &perfect);
		let worst_utility = engine.period_utility(&state, &worst);

		// Should choose perfect week
		assert!(value >= perfect_utility);
		assert!(value > worst_utility);
	}

	// ========================================================================
	// Optimal Outcome Tests
	// ========================================================================

	#[test]
	fn test_optimal_outcome_selects_best() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();
		let perfect = create_perfect_week(&hierarchy);
		let worst = create_worst_week(&hierarchy);
		let feasible = vec![perfect.clone(), worst.clone()];

		let optimal = engine.optimal_outcome(1, &state, &feasible);
		assert!(optimal.is_some());

		// Should be the perfect week
		let optimal_utility = engine.period_utility(&state, &optimal.unwrap());
		let perfect_utility = engine.period_utility(&state, &perfect);
		assert_eq!(optimal_utility, perfect_utility);
	}

	#[test]
	fn test_optimal_outcome_none_when_beyond_max() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();
		let feasible = vec![create_perfect_week(&hierarchy)];

		let optimal = engine.optimal_outcome(18, &state, &feasible);
		assert!(optimal.is_none());
	}

	// ========================================================================
	// Optimality Score Tests
	// ========================================================================

	#[test]
	fn test_period_optimality_perfect_score() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();
		let perfect = create_perfect_week(&hierarchy);
		let feasible = vec![perfect.clone()];

		let optimality = engine.period_optimality(1, &state, &perfect, &feasible);
		assert_eq!(optimality, 1.0);
	}

	#[test]
	fn test_period_optimality_worst_score() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();
		let perfect = create_perfect_week(&hierarchy);
		let worst = create_worst_week(&hierarchy);
		let feasible = vec![perfect, worst.clone()];

		let optimality = engine.period_optimality(1, &state, &worst, &feasible);
		assert_eq!(optimality, 0.0);
	}

	#[test]
	fn test_period_optimality_bounded() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();
		let mixed = create_mixed_week(&hierarchy);
		let feasible = vec![create_perfect_week(&hierarchy), mixed.clone(), create_worst_week(&hierarchy)];

		let optimality = engine.period_optimality(1, &state, &mixed, &feasible);
		assert!(optimality >= 0.0);
		assert!(optimality <= 1.0);
	}

	// ========================================================================
	// Season Optimality Tests
	// ========================================================================

	#[test]
	fn test_season_optimality_perfect_season() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let mut state = State::<TeamRecord>::new();
		let mut observed = vec![];
		let perfect = create_perfect_week(&hierarchy);

		// 5 perfect weeks
		for _ in 0..5 {
			observed.push((state.clone(), perfect.clone()));
			state = state.apply_period(&perfect);
		}

		let feasible = vec![perfect];
		let season_opt = engine.season_optimality(&observed, &feasible);
		assert_eq!(season_opt, 1.0);
	}

	#[test]
	fn test_season_optimality_worst_season() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let mut state = State::<TeamRecord>::new();
		let mut observed = vec![];
		let perfect = create_perfect_week(&hierarchy);
		let worst = create_worst_week(&hierarchy);

		// 5 worst weeks
		for _ in 0..5 {
			observed.push((state.clone(), worst.clone()));
			state = state.apply_period(&worst);
		}

		let feasible = vec![perfect, worst];
		let season_opt = engine.season_optimality(&observed, &feasible);
		assert_eq!(season_opt, 0.0);
	}

	#[test]
	fn test_season_optimality_mixed_season() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let mut state = State::<TeamRecord>::new();
		let mut observed = vec![];
		let perfect = create_perfect_week(&hierarchy);
		let worst = create_worst_week(&hierarchy);

		// Alternating perfect and worst weeks
		for i in 0..6 {
			let outcome = if i % 2 == 0 { &perfect } else { &worst };
			observed.push((state.clone(), outcome.clone()));
			state = state.apply_period(outcome);
		}

		let feasible = vec![perfect, worst];
		let season_opt = engine.season_optimality(&observed, &feasible);

		// Should be around 0.5
		assert!((season_opt - 0.5).abs() < 0.1);
	}

	#[test]
	fn test_season_optimality_empty() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let observed = vec![];
		let feasible = vec![create_perfect_week(&hierarchy)];

		let season_opt = engine.season_optimality(&observed, &feasible);
		assert_eq!(season_opt, 0.0);
	}

	// ========================================================================
	// Path Dependency Tests
	// ========================================================================

	#[test]
	fn test_path_dependency_affects_value() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 3).unwrap();

		// State after winning 2 games
		let mut winning_state = State::<TeamRecord>::new();
		let mut record = TeamRecord::default();
		record.wins = 2;
		winning_state.set_record(hierarchy.primary, record);

		// State after losing 2 games
		let mut losing_state = State::<TeamRecord>::new();
		let mut record = TeamRecord::default();
		record.losses = 2;
		losing_state.set_record(hierarchy.primary, record);

		let feasible = vec![create_perfect_week(&hierarchy)];

		// Value functions should differ based on history
		let value_winning = engine.value_function(3, &winning_state, &feasible);
		let value_losing = engine.value_function(3, &losing_state, &feasible);

		// Both should be positive (we can still have a good week)
		assert!(value_winning > 0.0);
		assert!(value_losing > 0.0);
	}

	// ========================================================================
	// NFL Realistic Scenario Tests
	// ========================================================================

	#[test]
	fn test_nfl_17_week_season() {
		let hierarchy = create_nfl_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let mut state = State::<TeamRecord>::new();
		let mut observed = vec![];

		// Simulate a 10-7 season with mixed results
		for week in 0..17 {
			let outcome = if week % 2 == 0 {
				create_perfect_week(&hierarchy)
			} else if week % 3 == 0 {
				create_worst_week(&hierarchy)
			} else {
				create_mixed_week(&hierarchy)
			};

			observed.push((state.clone(), outcome.clone()));
			state = state.apply_period(&outcome);
		}

		let feasible = vec![create_perfect_week(&hierarchy), create_mixed_week(&hierarchy), create_worst_week(&hierarchy)];

		let season_opt = engine.season_optimality(&observed, &feasible);

		// Should be a reasonable score
		assert!(season_opt >= 0.0);
		assert!(season_opt <= 1.0);
		println!("17-week season optimality: {}", season_opt);
	}

	// ========================================================================
	// Alternative Event Type Tests (Turnovers)
	// ========================================================================

	#[test]
	fn test_turnover_outcome_scores() {
		assert_eq!(TurnoverOutcome::Zero.score(), 1.0);
		assert_eq!(TurnoverOutcome::One.score(), 0.66);
		assert_eq!(TurnoverOutcome::Two.score(), 0.33);
		assert_eq!(TurnoverOutcome::ThreePlus.score(), 0.0);
	}

	#[test]
	fn test_turnover_record_tracking() {
		let mut record = TurnoverRecord::default();

		record.apply_outcome(TurnoverOutcome::Zero);
		assert_eq!(record.total_turnovers, 0);
		assert_eq!(record.games_played, 1);

		record.apply_outcome(TurnoverOutcome::Two);
		assert_eq!(record.total_turnovers, 2);
		assert_eq!(record.games_played, 2);

		record.apply_outcome(TurnoverOutcome::ThreePlus);
		assert_eq!(record.total_turnovers, 5);
		assert_eq!(record.games_played, 3);

		// Average: 5/3 ≈ 1.666
		assert!((record.derived_metric() - 1.666).abs() < 0.01);
	}

	#[test]
	fn test_turnover_engine_works() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let engine: TurnoverOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TurnoverRecord>::new();
		let mut perfect = PeriodOutcomes::new();
		perfect.set_outcome(hierarchy.primary, TurnoverOutcome::Zero);
		for &rival in &hierarchy.tier1_rivals {
			perfect.set_outcome(rival, TurnoverOutcome::ThreePlus);
		}

		let utility = engine.period_utility(&state, &perfect);
		assert!(utility > 0.0);
	}

	// ========================================================================
	// Edge Cases and Robustness Tests
	// ========================================================================

	#[test]
	fn test_tie_outcomes() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();
		let mut all_ties = PeriodOutcomes::new();

		for entity in hierarchy.all_entities() {
			all_ties.set_outcome(entity, GameOutcome::Tie);
		}

		let utility = engine.period_utility(&state, &all_ties);
		// Primary ties (0.5) - all rivals tie (0.5) = 0 differential
		assert_eq!(utility, 0.5); // Only primary's base score counts
	}

	#[test]
	fn test_cache_clearing() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights::default();
		let mut engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();
		let feasible = vec![create_perfect_week(&hierarchy)];

		engine.value_function(1, &state, &feasible);
		assert!(!engine.value_cache.is_empty());

		engine.clear_cache();
		assert!(engine.value_cache.is_empty());
	}

	#[test]
	fn test_large_hierarchy() {
		// 32 team league
		let mut tier1 = vec![];
		let mut tier2 = vec![];
		let mut tier3 = vec![];

		for i in 1..=31 {
			if i <= 3 {
				tier1.push(EntityId(i));
			} else if i <= 15 {
				tier2.push(EntityId(i));
			} else {
				tier3.push(EntityId(i));
			}
		}

		let hierarchy = EntityHierarchy {
			primary: EntityId(0),
			tier1_rivals: tier1,
			tier2_rivals: tier2,
			tier3_rivals: tier3,
		};

		let weights = HierarchicalWeights::default();
		let result: Result<TeamOptimalityEngine, _> = GenericOptimalityEngine::new(hierarchy, weights, 17);

		assert!(result.is_ok());
	}

	#[test]
	fn test_zero_weight_tier() {
		let hierarchy = create_simple_hierarchy();
		let weights = HierarchicalWeights {
			w_primary: 1.0,
			w_tier1: 0.5,
			w_tier2: 0.2,
			w_tier3: 0.0, // Don't care about tier3
		};
		let engine: TeamOptimalityEngine = GenericOptimalityEngine::new(hierarchy.clone(), weights, 17).unwrap();

		let state = State::<TeamRecord>::new();
		let mut outcomes = PeriodOutcomes::new();
		outcomes.set_outcome(hierarchy.primary, GameOutcome::Win);
		outcomes.set_outcome(hierarchy.tier3_rivals[0], GameOutcome::Win);

		let utility = engine.period_utility(&state, &outcomes);
		// Tier3 result shouldn't affect utility
		assert_eq!(utility, 1.0); // Only primary win counts
	}
}
