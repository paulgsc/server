//! `envctl resolve`: decide each "ours" value of one deployment file in a
//! terminal UI (CFG10, #606).
//!
//! One row per variable: what the file states, what the running container
//! uses, and whether they agree, with the variable's whole help beside it.
//! Every decision is held to the same rules `envctl check` holds the file to,
//! as it is made, so the file written on `w` passes the check.

use crate::{
	compose,
	deploy::{self, Deployment, Stated},
	import::{self, Row},
	schema::Schema,
	values::bound_text,
};
use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use ratatui::{
	crossterm::event::{self, Event, KeyCode, KeyEventKind},
	layout::{Constraint, Layout},
	style::{Color, Modifier, Style},
	text::{Line, Span},
	widgets::{Block, Borders, Cell, Paragraph, Row as TableRow, Table, TableState, Wrap},
	Frame,
};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
	Browse,
	Edit(String),
}

/// What a key did to the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
	Continue,
	Write,
	Leave,
}

pub(crate) struct App {
	title: String,
	source: String,
	rows: Vec<Row>,
	/// The decision per row, starting from what the file states.
	choices: Vec<Option<Stated>>,
	fixed: IndexMap<String, String>,
	defaults: BTreeMap<String, String>,
	selected: usize,
	mode: Mode,
	message: String,
	warned: bool,
}

impl App {
	pub(crate) fn new(title: String, source: String, schema: &Schema, rows: Vec<Row>, fixed: IndexMap<String, String>) -> Self {
		let choices = rows.iter().map(|row| row.file.clone()).collect();
		let defaults = schema.defaults().map(|(name, default)| (name.to_owned(), default.to_owned())).collect();
		Self {
			title,
			source,
			rows,
			choices,
			fixed,
			defaults,
			selected: 0,
			mode: Mode::Browse,
			message: String::new(),
			warned: false,
		}
	}

	fn current(&self, name: &str) -> Option<String> {
		if let Some(value) = self.fixed.get(name) {
			return Some(value.clone());
		}
		match self.rows.iter().position(|row| row.variable.name == name).and_then(|index| self.choices[index].as_ref()) {
			Some(Stated::Value(value)) => Some(value.clone()),
			Some(Stated::Unset) => None,
			None => self.defaults.get(name).cloned(),
		}
	}

	/// Why row `index`'s decision would fail `envctl check`.
	pub(crate) fn problems(&self, index: usize) -> Vec<String> {
		let row = &self.rows[index];
		let current = |name: &str| self.current(name);
		let mut problems = deploy::decided(&row.variable, self.choices[index].as_ref(), &current);
		if self.choices[index] == Some(Stated::Unset) && (row.variable.required || row.variable.default.is_some()) {
			problems.push(format!("{} cannot be left unset: state a value", row.variable.name));
		}
		problems
	}

	pub(crate) fn changed(&self) -> bool {
		self.rows.iter().zip(&self.choices).any(|(row, choice)| row.file != *choice)
	}

	fn status(&self, index: usize) -> (&'static str, Color) {
		let live = self.rows[index].running_statement();
		if !self.problems(index).is_empty() {
			("fix", Color::Red)
		} else if live.is_none() {
			("no container", Color::DarkGray)
		} else if live == self.choices[index] {
			("matches", Color::Green)
		} else {
			("differs", Color::Yellow)
		}
	}

	/// Handles one key; what the session should do next.
	pub(crate) fn key(&mut self, code: KeyCode) -> Outcome {
		if let Mode::Edit(text) = &mut self.mode {
			match code {
				KeyCode::Char(character) => text.push(character),
				KeyCode::Backspace => {
					text.pop();
				}
				KeyCode::Enter => {
					let value = std::mem::take(text);
					self.choices[self.selected] = Some(Stated::Value(value));
					self.mode = Mode::Browse;
				}
				KeyCode::Esc => self.mode = Mode::Browse,
				_ => {}
			}
			return Outcome::Continue;
		}
		self.message.clear();
		let row = &self.rows[self.selected];
		match code {
			KeyCode::Down | KeyCode::Char('j') => self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1)),
			KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
			KeyCode::Char('l') => match row.running_statement() {
				Some(live) => self.choices[self.selected] = Some(live),
				None => "no running container was read".clone_into(&mut self.message),
			},
			KeyCode::Char('L') => {
				for (index, row) in self.rows.iter().enumerate() {
					if let Some(live) = row.running_statement() {
						self.choices[index] = Some(live);
					}
				}
			}
			KeyCode::Char('d') => match &row.variable.default {
				Some(default) => self.choices[self.selected] = Some(Stated::Value(default.clone())),
				None => self.message = format!("{} has no default", row.variable.name),
			},
			KeyCode::Char('u') => {
				if row.variable.required || row.variable.default.is_some() {
					self.message = format!("{} cannot be left unset", row.variable.name);
				} else {
					self.choices[self.selected] = Some(Stated::Unset);
				}
			}
			KeyCode::Char('f') => self.choices[self.selected] = row.file.clone(),
			KeyCode::Char('e') | KeyCode::Enter => {
				let text = match &self.choices[self.selected] {
					Some(Stated::Value(value)) => value.clone(),
					_ => String::new(),
				};
				self.mode = Mode::Edit(text);
			}
			KeyCode::Char('w') => {
				let broken: Vec<&str> = (0..self.rows.len())
					.filter(|index| !self.problems(*index).is_empty())
					.map(|index| self.rows[index].variable.name.as_str())
					.collect();
				if broken.is_empty() || self.warned {
					return Outcome::Write;
				}
				self.message = format!("{} still fail the check; w again writes anyway", broken.join(", "));
				self.warned = true;
			}
			KeyCode::Char('q') | KeyCode::Esc => {
				if !self.changed() || self.warned {
					return Outcome::Leave;
				}
				"unsaved decisions: w writes them, q again leaves without".clone_into(&mut self.message);
				self.warned = true;
			}
			_ => {}
		}
		Outcome::Continue
	}

	/// The file's text with every decision that changed written in.
	pub(crate) fn written(&self, text: &str) -> String {
		let mut deployment = Deployment::parse(text);
		for (row, choice) in self.rows.iter().zip(&self.choices) {
			if let Some(choice) = choice.as_ref().filter(|choice| row.file.as_ref() != Some(*choice)) {
				deployment.set(&row.variable.name, choice, row.variable.group.as_deref());
			}
		}
		deployment.render()
	}

	fn draw(&self, frame: &mut Frame<'_>, state: &mut TableState) {
		let [table_area, detail_area, footer_area] = Layout::vertical([Constraint::Percentage(55), Constraint::Min(6), Constraint::Length(2)]).areas(frame.area());
		let shown = |statement: Option<&Stated>| match statement {
			None => "—".to_owned(),
			Some(Stated::Unset) => "#unset".to_owned(),
			Some(Stated::Value(value)) => value.clone(),
		};
		let rows = self.rows.iter().enumerate().map(|(index, row)| {
			let (status, color) = self.status(index);
			let changed = if row.file == self.choices[index] { "" } else { " *" };
			TableRow::new(vec![
				Cell::from(row.variable.name.clone()),
				Cell::from(format!("{}{changed}", shown(self.choices[index].as_ref()))),
				Cell::from(shown(row.running_statement().as_ref())),
				Cell::from(status).style(Style::default().fg(color)),
			])
		});
		let undecided = self.choices.iter().filter(|choice| choice.is_none()).count();
		let table = Table::new(
			rows,
			[
				Constraint::Percentage(30),
				Constraint::Percentage(30),
				Constraint::Percentage(28),
				Constraint::Percentage(12),
			],
		)
		.header(TableRow::new(vec!["Variable", "Decision", &self.source, "Status"]).style(Style::default().add_modifier(Modifier::BOLD)))
		.block(Block::default().borders(Borders::ALL).title(format!(" {} · {undecided} undecided ", self.title)))
		.row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
		.highlight_symbol("› ");
		state.select(Some(self.selected));
		frame.render_stateful_widget(table, table_area, state);

		let row = &self.rows[self.selected];
		let variable = &row.variable;
		let mut lines: Vec<Line<'_>> = variable.help.lines().map(|line| Line::from(line.to_owned())).collect();
		lines.push(Line::from(""));
		let mut facts = vec![format!("kind {:?}", variable.kind).to_lowercase()];
		facts.push(variable.default.as_ref().map_or_else(|| "no default".to_owned(), |default| format!("default {default}")));
		if let Some(condition) = &variable.required_when {
			facts.push(format!("required while {} is {}", condition.variable, condition.value));
		}
		if let Some(bounds) = &variable.bounds {
			facts.push(bound_text(bounds));
		}
		lines.push(Line::from(Span::styled(facts.join(" · "), Style::default().fg(Color::Cyan))));
		lines.extend(
			self
				.problems(self.selected)
				.into_iter()
				.map(|problem| Line::from(Span::styled(problem, Style::default().fg(Color::Red)))),
		);
		let detail = Paragraph::new(lines)
			.wrap(Wrap { trim: false })
			.block(Block::default().borders(Borders::ALL).title(format!(" {} ", variable.name)));
		frame.render_widget(detail, detail_area);

		let footer = match &self.mode {
			Mode::Edit(text) => format!("{} = {text}▏  Enter keeps it · Esc cancels", variable.name),
			Mode::Browse if !self.message.is_empty() => self.message.clone(),
			Mode::Browse => "↑↓ move · l take running · L take running for all · d default · u #unset · e edit · f file's · w write · q leave".to_owned(),
		};
		frame.render_widget(Paragraph::new(footer).wrap(Wrap { trim: true }), footer_area);
	}
}

/// The running container for `binary`, read, if one is.
fn live_for(binary: &str, container: Option<&str>, schemas: &[Schema]) -> Result<Option<(String, IndexMap<String, String>)>> {
	if let Some(container) = container {
		let (found, live) = import::inspect(container, schemas)?;
		if found != binary {
			bail!("container {container} runs {found}, not {binary}");
		}
		return Ok(Some((container.to_owned(), live)));
	}
	let Ok(running) = import::running() else { return Ok(None) };
	for container in running {
		if let Ok((found, live)) = import::inspect(&container, schemas) {
			if found == binary {
				return Ok(Some((container, live)));
			}
		}
	}
	Ok(None)
}

pub(crate) fn run(repo: &Path, schema: &Schema, role: &str, container: Option<&str>) -> Result<()> {
	let binary = &schema.binary;
	let path = repo.join("deploy").join(role).join(format!("{binary}.env"));
	let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
	let fixed = if deploy::COMPOSE_ROLES.contains(&role) {
		let raw = compose::config(repo).context("docker compose config failed, so compose's own values are unknown")?;
		let schemas = std::slice::from_ref(schema);
		deploy::labelled(schemas, &raw).0.get(binary).map(|(_, service)| compose::sets(service)).unwrap_or_default()
	} else {
		IndexMap::new()
	};
	let live = live_for(binary, container, std::slice::from_ref(schema))?;
	let source = live.as_ref().map_or_else(|| "no container".to_owned(), |(name, _)| name.clone());
	let rows = import::rows(schema, &text, &fixed, live.as_ref().map(|(_, live)| live));
	if rows.is_empty() {
		println!("{binary} has no \"ours\" variable for deploy/{role}/ to decide.");
		return Ok(());
	}
	let mut app = App::new(format!("deploy/{role}/{binary}.env"), source, schema, rows, fixed);
	let mut terminal = ratatui::try_init().context("opening the terminal")?;
	let mut state = TableState::default();
	let outcome = loop {
		if let Err(err) = terminal.draw(|frame| app.draw(frame, &mut state)) {
			let _ = ratatui::try_restore();
			return Err(err).context("drawing");
		}
		match event::read() {
			Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => match app.key(key.code) {
				Outcome::Continue => {}
				done => break done,
			},
			Ok(_) => {}
			Err(err) => {
				let _ = ratatui::try_restore();
				return Err(err).context("reading a key");
			}
		}
	};
	ratatui::try_restore().context("restoring the terminal")?;
	if outcome == Outcome::Write && app.changed() {
		std::fs::write(&path, app.written(&text)).with_context(|| format!("writing {}", path.display()))?;
		println!("wrote {}; review it with git diff, then commit.", path.strip_prefix(repo).unwrap_or(&path).display());
	} else {
		println!("{} unchanged.", path.strip_prefix(repo).unwrap_or(&path).display());
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{App, Outcome};
	use crate::{deploy::Stated, import::rows, schema::fixture::ours};
	use indexmap::IndexMap;
	use ratatui::{backend::TestBackend, crossterm::event::KeyCode, widgets::TableState, Terminal};

	fn app() -> (App, &'static str) {
		let text = "ON=false\nDAYS=30\n";
		let live = IndexMap::from([("ON".to_owned(), "true".to_owned()), ("URL".to_owned(), "https://x".to_owned())]);
		let fixed = IndexMap::from([("WIRE".to_owned(), "w".to_owned())]);
		let rows = rows(&ours(), text, &fixed, Some(&live));
		(App::new("deploy/home/probe.env".into(), "probe-1".into(), &ours(), rows, fixed), text)
	}

	#[test]
	fn keys_decide_rows_and_the_file_written_holds_them() {
		let (mut app, text) = app();
		assert!(!app.changed());
		assert_eq!(app.key(KeyCode::Char('L')), Outcome::Continue);
		assert_eq!(app.choices[0], Some(Stated::Value("true".into())), "ON takes what runs");
		app.key(KeyCode::Down);
		app.key(KeyCode::Down);
		app.key(KeyCode::Char('e'));
		for character in "nobody".chars() {
			app.key(KeyCode::Char(character));
		}
		app.key(KeyCode::Enter);
		assert_eq!(app.choices[2], Some(Stated::Value("nobody".into())));
		app.key(KeyCode::Down);
		app.key(KeyCode::Char('u'));
		assert!(app.message.contains("cannot be left unset"), "DAYS has a default");
		app.key(KeyCode::Char('d'));
		assert_eq!(app.choices[3], Some(Stated::Value("90".into())));
		assert_eq!(app.key(KeyCode::Char('w')), Outcome::Write);
		let written = app.written(text);
		let stated = crate::deploy::lines(&written).stated;
		assert_eq!(stated["ON"], Stated::Value("true".into()));
		assert_eq!(stated["URL"], Stated::Value("https://x".into()));
		assert_eq!(stated["WHO"], Stated::Value("nobody".into()));
	}

	#[test]
	fn a_decision_the_check_refuses_is_flagged_before_it_is_written() {
		let (mut app, _) = app();
		app.key(KeyCode::Char('e'));
		app.key(KeyCode::Backspace);
		app.key(KeyCode::Char('x'));
		app.key(KeyCode::Enter);
		assert!(app.problems(0).iter().any(|problem| problem.contains("must be true or false")), "{:?}", app.problems(0));
		assert_eq!(app.key(KeyCode::Char('w')), Outcome::Continue, "the first w warns");
		assert_eq!(app.key(KeyCode::Char('w')), Outcome::Write);
	}

	#[test]
	fn leaving_with_unsaved_decisions_asks_twice() {
		let (mut untouched, _) = app();
		assert_eq!(untouched.key(KeyCode::Char('q')), Outcome::Leave, "nothing decided, nothing to lose");
		let (mut decided, _) = app();
		decided.key(KeyCode::Char('l'));
		assert_eq!(decided.key(KeyCode::Char('q')), Outcome::Continue);
		assert_eq!(decided.key(KeyCode::Char('q')), Outcome::Leave);
	}

	#[test]
	fn the_screen_shows_each_row_its_running_value_and_the_help() {
		let (app, _) = app();
		let mut terminal = Terminal::new(TestBackend::new(110, 30)).unwrap();
		let mut state = TableState::default();
		terminal.draw(|frame| app.draw(frame, &mut state)).unwrap();
		let screen: String = terminal.backend().buffer().content().iter().map(ratatui::buffer::Cell::symbol).collect();
		for expected in ["deploy/home/probe.env", "probe-1", "ON", "differs", "ON's help.", "URL"] {
			assert!(screen.contains(expected), "{expected} missing from the screen");
		}
		assert!(!screen.contains("WIRE"), "compose's own wiring is not the file's to decide");
	}
}
