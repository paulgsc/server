# Utility MDP Engine (Boyo's Attempt at Math™)

**Disclaimer:** This is all pure hallucination from boyo with heavy AI hand-holding. If you're reading this (probably future me), I was just trying to formalize some vibes into code. Don't @ me about rigor.

---

## What Even Is This?

This crate is a **deterministic, compile-time friendly MDP engine** for tracking utility over discrete events. Translation: it's for answering "how optimal were the outcomes?" when you've got:

- Small finite systems (≤ 32 teams/entities)
- Discrete bounded events (wins/losses, interceptions, whatever)
- A need to compare what happened vs. what could've happened

Think: sports seasons, player stats, or any situation where you want to track whether reality was kind to you.

---

## The Core Machinery

### State

A **state** $\mathbf{R}_w$ is just a vector of cumulative records at week $w$:

$$\mathbf{R}_w = (R_w(t_1), R_w(t_2), \ldots, R_w(t_N))$$

Each component is a small integer counter (wins, losses, interceptions, whatever you're tracking).

### Events

An **event** $e_w$ is a discrete outcome vector across $N$ entities:

$$e_w = (e_w(t_1), e_w(t_2), \ldots, e_w(t_N)), \quad e_w(t) \in \{\text{finite bounded set}\}$$

Examples:
- Game outcomes: $\{0, 0.5, 1\}$ (loss, tie, win)
- Interceptions: $\{0, 1, 2, 3+\}$
- Injuries: $\{0, 1, 2+\}$ star players out

**Key insight:** The engine doesn't care what the events *are*, just that they're discrete and bounded.

### Transition

States update deterministically each week:

$$\mathbf{R}_w = \mathbf{R}_{w-1} \oplus e_w$$

No randomness, no chaos — just pure accumulation.

### Utility

A **utility function** $U(\mathbf{R}_w, e_w)$ assigns value to outcomes:

$$U(\mathbf{R}_w, e_w) = \sum_{i=1}^m \alpha_i f_i(\mathbf{R}_w, e_w)$$

Where each $f_i$ captures some aspect you care about:
- Your team's performance
- Rival teams doing poorly
- Conference/division hierarchies
- Whatever vibes you want to formalize

### Optimality (The Actual Point)

Using **backward induction**, we compute the optimal path of events that maximizes cumulative utility.

**Week-level optimality:**

$$\text{Opt}_w = \frac{U(\mathbf{R}_{w-1} \oplus e_w^{\text{obs}}, e_w^{\text{obs}}) + V_{w+1}(\mathbf{R}_{w-1} \oplus e_w^{\text{obs}})}{V_w(\mathbf{R}_{w-1})}$$

**Season-level optimality:**

$$\text{Opt}_{\text{season}} = \frac{1}{W} \sum_{w=1}^W \text{Opt}_w$$

This gives you a score in $[0,1]$ for how close reality came to optimal.

---

## Why This Structure?

The formalization handles these gnarly bits:

1. **Path dependence**: Week 10's optimal outcome depends on what happened in weeks 1-9 (Markovian)
2. **Hierarchy**: Divisional rivals matter more than conference rivals matter more than random teams
3. **Multiple events**: Track games, injuries, turnovers — whatever, simultaneously
4. **Longitudinal tracking**: Watch utility grow/decay over a season like a health bar

---

## Key Features

- **Deterministic & bounded**: Small integer states, finite event sets, no surprises
- **Generic over event types**: Works for any discrete bounded metric
- **Path-dependent**: History matters (this is an MDP, not independent weeks)
- **Compile-time friendly**: Precompute DP tables for small $N$
- **Utility growth tracking**: Get longitudinal charts of vibes over time

---

## Example Use Cases

**Sports:**
- "How optimal was the 49ers' season given what actually happened?"
- "If my QB threw 2 picks but everyone else threw 3+, did we still come out ahead?"

**Generic discrete systems:**
- Task completions, project milestones, anything with $N \leq 32$ entities and bounded outcomes

---

## The Formalism (Speedrun)

| Concept | Math | Plain English |
|---------|------|---------------|
| **State** | $\mathbf{R}_w = (R_w(t_1), \ldots, R_w(t_N))$ | Cumulative records |
| **Event** | $e_w = (e_w(t_1), \ldots, e_w(t_N))$ | This week's outcomes |
| **Transition** | $\mathbf{R}_w = \mathbf{R}_{w-1} \oplus e_w$ | Add event to state |
| **Utility** | $U(\mathbf{R}_w, e_w)$ | How good is this? |
| **Optimal path** | $\max V_w(\mathbf{R}_{w-1})$ via backward induction | Best possible timeline |
| **Optimality score** | $\text{Opt}_w \in [0,1]$ | Reality vs. optimal |

---

## TL;DR

This is a formalized way to answer: **"Given what happened, how close were we to the best possible outcomes?"**

It's:
- Fully deterministic
- Generic over discrete event types
- Path-dependent (MDP structure)
- Small-scale friendly (≤ 32 entities)
- Designed for compile-time computation

Future me: this was a genuine attempt to make "vibes" rigorous. The math is sound even if the motivation is silly.

---

## Notes for Future Me

- Don't try to scale this to infinite systems
- The "optimality" is relative to your utility function — GIGO applies
- If you're confused about the backward induction, just remember: we're computing "best path from here" by working backwards from the end
- The Markovian property just means "future optimal depends only on current state, not full history" — but state *accumulates* history, so it's fine
