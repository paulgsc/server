//! `envctl prune` (`ENV2b`, #628): a role's variables in a checklist, grouped
//! by the schema's sections and shown a page at a time, to prune some through
//! sops. Names and states are shown, never a value.
//!
//! The checklist is a pure model ([`Model`]: keys in, a screen out), so its
//! behaviour is tested without a terminal; [`run`] is the thin terminal layer
//! over it.

use crate::{
	preflight::first_line,
	schema::Schema,
	sops::{self, Plain},
};
use anyhow::{bail, Context, Result};
use std::{fmt::Write as _, path::Path};

/// The group of names the binary no longer declares.
const UNDECLARED: &str = "not declared";

/// One variable of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Item {
	pub(crate) name: String,
	/// Whether the binary declares it: pruning one it does states it unset,
	/// pruning one it does not removes the line.
	pub(crate) declared: bool,
	/// Whether the file states it unset (an empty value) already.
	pub(crate) unset: bool,
	pub(crate) help: String,
	pub(crate) ticked: bool,
}

/// A key the checklist answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Key {
	Up,
	Down,
	Left,
	Right,
	NextGroup,
	PreviousGroup,
	Toggle,
	Enter,
	Yes,
	Quit,
	/// Any other key: at the review, it goes back.
	Other,
}

/// What the checklist asks for after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
	Continue,
	Write,
	Quit,
}

/// The checklist: its groups, each a list of indexes into `items`, and where
/// the cursor is.
#[derive(Debug)]
pub(crate) struct Model {
	pub(crate) items: Vec<Item>,
	groups: Vec<(String, Vec<usize>)>,
	group: usize,
	page: usize,
	/// The row on the page.
	row: usize,
	page_size: usize,
	/// The rows a page asks for (`--page`), before the terminal's height.
	wanted: usize,
	/// The terminal's width, to cut each line to; 0 for no limit.
	width: usize,
	reviewing: bool,
}

impl Model {
	/// The file's variables, grouped by `schema`'s sections in its order, and
	/// those it does not declare last.
	pub(crate) fn new(schema: &Schema, file: &Plain, page_size: usize) -> Self {
		let mut items = Vec::new();
		let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
		let mut push = |group: &str, item: Item, items: &mut Vec<Item>| {
			let index = items.len();
			items.push(item);
			match groups.iter_mut().find(|(name, _)| name == group) {
				Some((_, members)) => members.push(index),
				None => groups.push((group.to_owned(), vec![index])),
			}
		};
		for variable in &schema.variables {
			let Some(value) = file.get(&variable.name) else { continue };
			let item = Item {
				name: variable.name.clone(),
				declared: true,
				unset: value.is_empty(),
				help: first_line(&variable.help),
				ticked: false,
			};
			push(variable.group.as_deref().unwrap_or("Other"), item, &mut items);
		}
		for (name, value) in file {
			if schema.variable(name).is_none() {
				let item = Item {
					name: name.clone(),
					declared: false,
					unset: value.is_empty(),
					help: format!("{} does not read it", schema.binary),
					ticked: false,
				};
				push(UNDECLARED, item, &mut items);
			}
		}
		Self {
			items,
			groups,
			group: 0,
			page: 0,
			row: 0,
			page_size: page_size.max(1),
			wanted: page_size.max(1),
			width: 0,
			reviewing: false,
		}
	}

	/// Fit to a terminal of `width` × `height`: each line cut to the width,
	/// and a page no taller than what is left beside the header and keys, so
	/// the cursor's row is always on screen (Codex on #629). The cursor keeps
	/// its item.
	pub(crate) fn fit(&mut self, width: usize, height: usize) {
		self.width = width;
		let rows = self.wanted.min(height.saturating_sub(5)).max(1);
		if rows != self.page_size {
			let at = self.page * self.page_size + self.row;
			self.page_size = rows;
			self.page = at / rows;
			self.row = at % rows;
		}
	}

	fn members(&self) -> &[usize] {
		self.groups.get(self.group).map_or(&[][..], |(_, members)| members)
	}

	fn pages(&self) -> usize {
		self.members().len().div_ceil(self.page_size).max(1)
	}

	/// The indexes on the current page.
	fn page_items(&self) -> &[usize] {
		let members = self.members();
		let start = (self.page * self.page_size).min(members.len());
		&members[start..(start + self.page_size).min(members.len())]
	}

	/// The names ticked, in the file's order of groups.
	pub(crate) fn ticked(&self) -> Vec<&Item> {
		self
			.groups
			.iter()
			.flat_map(|(_, members)| members.iter().map(|&index| &self.items[index]))
			.filter(|item| item.ticked)
			.collect()
	}

	/// The checklist after `key`, and what it asks for.
	pub(crate) fn key(&mut self, key: Key) -> Step {
		if self.reviewing {
			return match key {
				Key::Yes => Step::Write,
				Key::Quit => Step::Quit,
				_ => {
					self.reviewing = false;
					Step::Continue
				}
			};
		}
		let on_page = self.page_items().len();
		match key {
			Key::Up if self.row > 0 => self.row -= 1,
			Key::Up if self.page > 0 => {
				self.page -= 1;
				self.row = self.page_size - 1;
			}
			Key::Down if self.row + 1 < on_page => self.row += 1,
			Key::Down | Key::Right if self.page + 1 < self.pages() => {
				self.page += 1;
				self.row = 0;
			}
			Key::Left if self.page > 0 => {
				self.page -= 1;
				self.row = 0;
			}
			Key::NextGroup if !self.groups.is_empty() => {
				self.group = (self.group + 1) % self.groups.len();
				self.page = 0;
				self.row = 0;
			}
			Key::PreviousGroup if !self.groups.is_empty() => {
				self.group = (self.group + self.groups.len() - 1) % self.groups.len();
				self.page = 0;
				self.row = 0;
			}
			Key::Toggle => {
				if let Some(&index) = self.page_items().get(self.row) {
					self.items[index].ticked = !self.items[index].ticked;
				}
			}
			Key::Enter if !self.ticked().is_empty() => self.reviewing = true,
			Key::Quit => return Step::Quit,
			_ => {}
		}
		Step::Continue
	}

	/// The screen, each line cut to the terminal's width.
	pub(crate) fn render(&self, file: &str) -> String {
		let screen = self.screen(file);
		if self.width == 0 {
			return screen;
		}
		screen.lines().map(|line| line.chars().take(self.width).collect::<String>() + "\n").collect()
	}

	/// The group and page, its rows, and the keys; or the review.
	fn screen(&self, file: &str) -> String {
		let mut out = String::new();
		if self.reviewing {
			let _ = writeln!(out, "{file}: prune {} variable(s)?\n", self.ticked().len());
			for item in self.ticked() {
				let what = if item.declared {
					"state unset (an empty value; the binary takes its default)"
				} else {
					"remove the line"
				};
				let _ = writeln!(out, "  {:<36} {what}", item.name);
			}
			out.push_str("\n  y  write, through sops      any other key  back      q  leave, nothing written\n");
			return out;
		}
		let (group, members) = self.groups.get(self.group).map_or(("", &[][..]), |(name, members)| (name.as_str(), members.as_slice()));
		let _ = writeln!(
			out,
			"{file}   group {}/{}: {group} ({} variable(s))   page {}/{}   ticked {}\n",
			self.group + 1,
			self.groups.len(),
			members.len(),
			self.page + 1,
			self.pages(),
			self.ticked().len()
		);
		for (row, &index) in self.page_items().iter().enumerate() {
			let item = &self.items[index];
			let cursor = if row == self.row { '>' } else { ' ' };
			let tick = if item.ticked { 'x' } else { ' ' };
			let state = if item.unset { "unset" } else { "value" };
			let _ = writeln!(out, "{cursor} [{tick}] {:<36} {state:<6} {}", item.name, item.help);
		}
		out.push_str("\n  up/down move   space tick   left/right page   tab/shift-tab group   enter review   q leave\n");
		out
	}
}

/// `file` with `ticked` pruned: a declared one stated unset (empty), any
/// other's line removed.
pub(crate) fn apply(file: &Plain, ticked: &[&Item]) -> Plain {
	let mut out = file.clone();
	for item in ticked {
		if item.declared {
			out.insert(item.name.clone(), String::new());
		} else {
			out.shift_remove(&item.name);
		}
	}
	out
}

/// `envctl prune`: decrypt into memory, choose (in the checklist, or
/// `names`), and write back through sops.
pub(crate) fn prune(repo: &Path, schema: &Schema, role: &str, names: &[String], page_size: usize, sops_program: &[String]) -> Result<()> {
	let relative = sops::relative(role, &schema.binary);
	if !repo.join(&relative).exists() {
		bail!("{} does not exist; `make import` writes it", relative.display());
	}
	let file = sops::decrypt(sops_program, repo, &relative)?;
	let mut model = Model::new(schema, &file, page_size);
	let shown = relative.display().to_string();
	if names.is_empty() {
		if !terminal::run(&mut model, &shown)? {
			println!("{shown}: nothing written.");
			return Ok(());
		}
	} else {
		for name in names {
			let Some(item) = model.items.iter_mut().find(|item| &item.name == name) else {
				bail!("{shown} states no {name}");
			};
			item.ticked = true;
		}
	}
	let ticked = model.ticked();
	if ticked.is_empty() {
		println!("{shown}: nothing ticked, nothing written.");
		return Ok(());
	}
	let pruned = apply(&file, &ticked);
	let encrypted = sops::encrypt(sops_program, repo, &relative, &pruned)?;
	std::fs::write(repo.join(&relative), encrypted).with_context(|| format!("writing {shown}"))?;
	println!("{shown}: pruned {}.", ticked.iter().map(|item| item.name.as_str()).collect::<Vec<_>>().join(", "));
	println!("Next: make preflight, then make up; commit {shown}.");
	Ok(())
}

mod terminal {
	//! The checklist in a terminal: raw mode on the alternate screen, put
	//! back however it ends.
	use super::{Key, Model, Step};
	use anyhow::{Context, Result};
	use crossterm::{
		cursor,
		event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
		execute,
		terminal::{self, ClearType},
	};
	use std::io::{stdout, Write as _};

	/// Puts the terminal back when dropped, an error or a panic included.
	struct Raw;

	impl Drop for Raw {
		fn drop(&mut self) {
			let _ = execute!(stdout(), terminal::LeaveAlternateScreen, cursor::Show);
			let _ = terminal::disable_raw_mode();
		}
	}

	/// Whether the owner chose to write.
	pub(super) fn run(model: &mut Model, file: &str) -> Result<bool> {
		terminal::enable_raw_mode().context("the checklist needs a terminal; pass --prune NAME,... instead")?;
		let _raw = Raw;
		execute!(stdout(), terminal::EnterAlternateScreen, cursor::Hide)?;
		loop {
			if let Ok((width, height)) = terminal::size() {
				model.fit(usize::from(width), usize::from(height));
			}
			let screen = model.render(file).replace('\n', "\r\n");
			execute!(stdout(), cursor::MoveTo(0, 0), terminal::Clear(ClearType::All))?;
			print!("{screen}");
			stdout().flush()?;
			let Event::Key(pressed) = event::read()? else { continue };
			if pressed.kind != KeyEventKind::Press {
				continue;
			}
			let key = match pressed.code {
				KeyCode::Up | KeyCode::Char('k') => Key::Up,
				KeyCode::Down | KeyCode::Char('j') => Key::Down,
				KeyCode::Left | KeyCode::Char('h') => Key::Left,
				KeyCode::Right | KeyCode::Char('l') => Key::Right,
				KeyCode::BackTab => Key::PreviousGroup,
				KeyCode::Tab if pressed.modifiers.contains(KeyModifiers::SHIFT) => Key::PreviousGroup,
				KeyCode::Tab => Key::NextGroup,
				KeyCode::Char(' ') => Key::Toggle,
				KeyCode::Enter => Key::Enter,
				KeyCode::Char('y' | 'Y') => Key::Yes,
				KeyCode::Char('q') | KeyCode::Esc => Key::Quit,
				KeyCode::Char('c') if pressed.modifiers.contains(KeyModifiers::CONTROL) => Key::Quit,
				// Not ignored: at the review, any other key goes back, as it says.
				_ => Key::Other,
			};
			match model.key(key) {
				Step::Continue => {}
				Step::Write => return Ok(true),
				Step::Quit => return Ok(false),
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{apply, Key, Model, Step};
	use crate::{
		schema::{
			fixture::{schema, var},
			Kind,
		},
		sops::Plain,
	};

	fn model(page_size: usize) -> (Model, Plain) {
		let mut variables = Vec::new();
		for (name, group) in [("A1", "A"), ("A2", "A"), ("A3", "A"), ("B1", "B")] {
			let mut variable = var(name, Kind::String);
			variable.group = Some(group.into());
			variables.push(variable);
		}
		let probe = schema("probe", variables);
		let file: Plain = [("A1", "x"), ("A2", ""), ("A3", "y"), ("B1", "z"), ("OLD", "w")]
			.into_iter()
			.map(|(name, value)| (name.to_owned(), value.to_owned()))
			.collect();
		(Model::new(&probe, &file, page_size), file)
	}

	#[test]
	fn groups_follow_the_schema_and_undeclared_names_come_last() {
		let (mut model, _) = model(2);
		let first = model.render("f");
		assert!(first.contains("group 1/3: A (3 variable(s))   page 1/2"), "{first}");
		assert!(first.contains("> [ ] A1") && first.contains("  [ ] A2") && !first.contains("A3"), "{first}");
		assert!(first.contains("A2                                   unset"), "an empty value reads as unset: {first}");
		model.key(Key::Right);
		assert!(model.render("f").contains("page 2/2") && model.render("f").contains("> [ ] A3"));
		model.key(Key::NextGroup);
		model.key(Key::NextGroup);
		let last = model.render("f");
		assert!(
			last.contains("group 3/3: not declared") && last.contains("OLD") && last.contains("probe does not read it"),
			"{last}"
		);
		model.key(Key::NextGroup);
		assert!(model.render("f").contains("group 1/3: A"), "tab wraps around");
		model.key(Key::PreviousGroup);
		assert!(model.render("f").contains("group 3/3"), "shift-tab wraps back");
	}

	#[test]
	fn down_crosses_pages_and_up_comes_back() {
		let (mut model, _) = model(2);
		model.key(Key::Down);
		model.key(Key::Down);
		assert!(model.render("f").contains("page 2/2") && model.render("f").contains("> [ ] A3"));
		model.key(Key::Up);
		assert!(model.render("f").contains("page 1/2") && model.render("f").contains("> [ ] A2"));
	}

	#[test]
	fn ticks_are_reviewed_then_written_and_a_value_is_never_shown() {
		let (mut model, file) = model(10);
		assert_eq!(model.key(Key::Enter), Step::Continue, "nothing ticked, nothing to review");
		model.key(Key::Toggle);
		model.key(Key::NextGroup);
		model.key(Key::NextGroup);
		model.key(Key::Toggle);
		assert_eq!(model.key(Key::Enter), Step::Continue);
		let review = model.render("f");
		assert!(
			review.contains("prune 2 variable(s)") && review.contains("A1") && review.contains("state unset"),
			"{review}"
		);
		assert!(review.contains("OLD") && review.contains("remove the line"), "{review}");
		for value in ["x", "y", "z", "w"] {
			assert!(!review.contains(&format!(" {value}\n")) && !model.render("f").contains(&format!("={value}")));
		}
		assert_eq!(model.key(Key::Left), Step::Continue, "any other key goes back");
		model.key(Key::Enter);
		assert_eq!(model.key(Key::Yes), Step::Write);
		let pruned = apply(&file, &model.ticked());
		assert_eq!(pruned.get("A1").map(String::as_str), Some(""), "a declared one is stated unset");
		assert!(!pruned.contains_key("OLD"), "an undeclared one is removed");
		assert_eq!(pruned.get("A3").map(String::as_str), Some("y"));
	}

	#[test]
	fn any_other_key_at_the_review_goes_back_and_y_then_writes_nothing_unasked() {
		let (mut model, _) = model(10);
		model.key(Key::Toggle);
		model.key(Key::Enter);
		assert!(model.render("f").contains("prune 1 variable(s)?"));
		assert_eq!(model.key(Key::Other), Step::Continue, "`n`, or any key the checklist does not use");
		assert!(model.render("f").contains("> [x] A1"), "back to the checklist");
		assert_eq!(model.key(Key::Yes), Step::Continue, "y outside the review writes nothing");
	}

	#[test]
	fn the_screen_fits_the_terminal_and_the_cursor_keeps_its_item() {
		let (mut model, _) = model(10);
		model.key(Key::Down);
		model.key(Key::Down);
		model.fit(80, 7);
		let screen = model.render("f");
		assert!(screen.lines().count() <= 7, "no taller than the terminal: {screen}");
		assert!(screen.contains("page 2/2") && screen.contains("> [ ] A3"), "the cursor keeps A3 on screen: {screen}");
		model.fit(30, 7);
		let narrow = model.render("deploy/home/probe.sops.env");
		assert!(narrow.lines().all(|line| line.chars().count() <= 30), "no wider than the terminal: {narrow}");
	}

	#[test]
	fn q_leaves() {
		let (mut model, _) = model(10);
		assert_eq!(model.key(Key::Quit), Step::Quit);
	}
}
