//! A small max-flow (Dinic), enough for one conference's remaining games.
//!
//! The graph is source → game → each of its two teams → sink. A game carries
//! the two points it hands out; a team's edge to the sink carries how many
//! points it may still take. Every game's points placed means every team can
//! finish at or under its cap at once.

use std::collections::VecDeque;

#[derive(Debug, Clone, Copy)]
struct Edge {
	to: usize,
	cap: u32,
}

#[derive(Debug)]
pub(crate) struct Network {
	edges: Vec<Edge>,
	adj: Vec<Vec<usize>>,
}

impl Network {
	pub(crate) fn new(nodes: usize) -> Self {
		Self {
			edges: Vec::new(),
			adj: vec![Vec::new(); nodes],
		}
	}

	/// Adds `from → to` with `cap`, and returns the edge's id for [`Self::flow_on`].
	pub(crate) fn add(&mut self, from: usize, to: usize, cap: u32) -> usize {
		let id = self.edges.len();
		self.edges.push(Edge { to, cap });
		self.adj[from].push(id);
		self.edges.push(Edge { to: from, cap: 0 });
		self.adj[to].push(id + 1);
		id
	}

	/// Flow pushed along edge `id` (its reverse edge's residual).
	pub(crate) fn flow_on(&self, id: usize) -> u32 {
		self.edges[id ^ 1].cap
	}

	pub(crate) fn max_flow(&mut self, source: usize, sink: usize) -> u32 {
		let mut total = 0;
		while let Some(level) = self.levels(source, sink) {
			let mut next = vec![0usize; self.adj.len()];
			loop {
				let pushed = self.push(source, sink, u32::MAX, &level, &mut next);
				if pushed == 0 {
					break;
				}
				total += pushed;
			}
		}
		total
	}

	/// Nodes still reachable from `source` in the residual graph. After
	/// [`Self::max_flow`] this is the source side of a minimum cut.
	pub(crate) fn reachable(&self, source: usize) -> Vec<bool> {
		let mut seen = vec![false; self.adj.len()];
		seen[source] = true;
		let mut queue = VecDeque::from([source]);
		while let Some(u) = queue.pop_front() {
			for &e in &self.adj[u] {
				let Edge { to, cap } = self.edges[e];
				if cap > 0 && !seen[to] {
					seen[to] = true;
					queue.push_back(to);
				}
			}
		}
		seen
	}

	fn levels(&self, source: usize, sink: usize) -> Option<Vec<u32>> {
		let mut level = vec![u32::MAX; self.adj.len()];
		level[source] = 0;
		let mut queue = VecDeque::from([source]);
		while let Some(u) = queue.pop_front() {
			for &e in &self.adj[u] {
				let Edge { to, cap } = self.edges[e];
				if cap > 0 && level[to] == u32::MAX {
					level[to] = level[u] + 1;
					queue.push_back(to);
				}
			}
		}
		(level[sink] != u32::MAX).then_some(level)
	}

	fn push(&mut self, u: usize, sink: usize, limit: u32, level: &[u32], next: &mut [usize]) -> u32 {
		if u == sink {
			return limit;
		}
		while next[u] < self.adj[u].len() {
			let e = self.adj[u][next[u]];
			let Edge { to, cap } = self.edges[e];
			if cap > 0 && level[to] == level[u] + 1 {
				let pushed = self.push(to, sink, limit.min(cap), level, next);
				if pushed > 0 {
					self.edges[e].cap -= pushed;
					self.edges[e ^ 1].cap += pushed;
					return pushed;
				}
			}
			next[u] += 1;
		}
		0
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn finds_the_bottleneck() {
		let mut n = Network::new(4);
		n.add(0, 1, 3);
		n.add(0, 2, 2);
		n.add(1, 2, 5);
		let out = n.add(1, 3, 2);
		n.add(2, 3, 3);
		assert_eq!(n.max_flow(0, 3), 5);
		assert_eq!(n.flow_on(out), 2);
		assert_eq!(n.reachable(0), [true, false, false, false]);
	}
}
