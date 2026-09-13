//! The TUI line editor: a small, dependency-free, readline-flavored input
//! buffer with history. Pure logic — no terminal access — so every behavior
//! is unit-testable. Multi-line input is supported (Alt+Enter inserts a
//! newline); Up/Down browse history only while the buffer is single-line,
//! matching the "type, recall, edit" flow of every terminal REPL.

#[derive(Debug, Default)]
pub struct Input {
    buf: Vec<char>,
    cursor: usize, // char index into buf
    history: Vec<String>,
    hist_idx: Option<usize>,
    /// Buffer content saved when history browsing started.
    draft: String,
}

/// Prompt gutter: first line gets `> `, continuations get two spaces so
/// wrapped text aligns under typed text.
const PREFIX_FIRST: &str = "> ";

impl Input {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn text(&self) -> String {
        self.buf.iter().collect()
    }

    /// Single-line = no embedded newline (history browsing allowed).
    pub fn is_single_line(&self) -> bool {
        !self.buf.contains(&'\n')
    }

    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            let c = if c == '\r' { '\n' } else { c };
            self.buf.insert(self.cursor, c);
            self.cursor += 1;
        }
        self.exit_history();
    }

    /// Alt+Enter: insert a literal newline.
    pub fn newline(&mut self) {
        self.insert_str("\n");
    }

    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.buf.remove(self.cursor);
        self.exit_history();
        true
    }

    pub fn delete(&mut self) -> bool {
        if self.cursor >= self.buf.len() {
            return false;
        }
        self.buf.remove(self.cursor);
        self.exit_history();
        true
    }

    pub fn left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        true
    }

    pub fn right(&mut self) -> bool {
        if self.cursor >= self.buf.len() {
            return false;
        }
        self.cursor += 1;
        true
    }

    pub fn home(&mut self) {
        // Home goes to the start of the current logical line.
        while self.cursor > 0 && self.buf[self.cursor - 1] != '\n' {
            self.cursor -= 1;
        }
    }

    pub fn end(&mut self) {
        while self.cursor < self.buf.len() && self.buf[self.cursor] != '\n' {
            self.cursor += 1;
        }
    }

    /// Ctrl+W: delete the word before the cursor (readline-style: the run
    /// of non-space chars immediately left, plus any spaces before it).
    pub fn delete_word_left(&mut self) -> bool {
        let start = self.word_left_boundary();
        if start == self.cursor {
            return false;
        }
        self.buf.drain(start..self.cursor);
        self.cursor = start;
        self.exit_history();
        true
    }

    /// Ctrl+U: clear from cursor to start of the current logical line.
    pub fn clear_to_line_start(&mut self) -> bool {
        let mut start = self.cursor;
        while start > 0 && self.buf[start - 1] != '\n' {
            start -= 1;
        }
        if start == self.cursor {
            return false;
        }
        self.buf.drain(start..self.cursor);
        self.cursor = start;
        self.exit_history();
        true
    }

    fn word_left_boundary(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && self.buf[i - 1].is_whitespace() && self.buf[i - 1] != '\n' {
            i -= 1;
        }
        while i > 0 && !self.buf[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    pub fn cursor_char(&self) -> usize {
        self.cursor
    }

    /// Move the cursor to the end of the previous logical line.
    pub fn line_up(&mut self) -> bool {
        if self.cursor == 0 || !self.buf[..self.cursor].contains(&'\n') {
            return false;
        }
        // Step over the newline we're after, then to the end of that line.
        let mut i = self.cursor;
        while i > 0 && self.buf[i - 1] != '\n' {
            i -= 1;
        }
        i -= 1; // the newline
        self.cursor = i;
        true
    }

    /// Move the cursor to the start (then end) of the next logical line.
    pub fn line_down(&mut self) -> bool {
        let Some(nl) = self.buf[self.cursor..].iter().position(|&c| c == '\n') else {
            return false;
        };
        self.cursor += nl + 1;
        true
    }

    /// Up: previous history entry (saves the in-progress draft first).
    /// Returns false when there is nothing older to show, or the buffer is
    /// multi-line (Up then means "move within the text", handled by caller).
    pub fn history_prev(&mut self) -> bool {
        if !self.is_single_line() {
            return false;
        }
        let next = match self.hist_idx {
            None if self.history.is_empty() => return false,
            None => self.history.len() - 1,
            Some(0) => return false,
            Some(i) => i - 1,
        };
        if self.hist_idx.is_none() {
            self.draft = self.text();
        }
        self.hist_idx = Some(next);
        self.load_history_entry(next);
        true
    }

    /// Down: next history entry; past the newest restores the saved draft.
    pub fn history_next(&mut self) -> bool {
        let Some(i) = self.hist_idx else {
            return false;
        };
        if i + 1 >= self.history.len() {
            self.hist_idx = None;
            self.buf = self.draft.chars().collect();
            self.cursor = self.buf.len();
            return true;
        }
        self.hist_idx = Some(i + 1);
        self.load_history_entry(i + 1);
        true
    }

    fn load_history_entry(&mut self, i: usize) {
        self.buf = self.history[i].chars().collect();
        self.cursor = self.buf.len();
    }

    fn exit_history(&mut self) {
        if self.hist_idx.is_some() {
            self.hist_idx = None;
            self.draft.clear();
        }
    }

    /// Take the buffer as a submission (None when blank). Pushes non-blank
    /// entries onto history, resets state.
    pub fn submit(&mut self) -> Option<String> {
        while self.buf.last() == Some(&'\n') {
            self.buf.pop();
        }
        let text = self.text();
        if text.trim().is_empty() {
            self.buf.clear();
            self.cursor = 0;
            self.exit_history();
            return None;
        }
        if self.history.last().map(String::as_str) != Some(text.as_str()) {
            self.history.push(text.clone());
        }
        self.buf.clear();
        self.cursor = 0;
        self.hist_idx = None;
        self.draft.clear();
        Some(text)
    }

    /// Load persistent history (newest last); caps at 500 entries.
    pub fn load_history(&mut self, raw: &str) {
        self.history = raw.lines().map(str::to_string).collect();
        let len = self.history.len();
        if len > 500 {
            self.history.drain(..len - 500);
        }
    }

    pub fn history_text(&self) -> String {
        let mut s = self.history.join("\n");
        if !s.is_empty() {
            s.push('\n');
        }
        s
    }

    /// Render into display lines (prompt gutter added by the caller), hard
    ///-wrapping at `width` columns; embedded newlines force breaks. Returns
    /// the lines and the cursor's (row, col). At least one line is always
    /// produced so the cursor has somewhere to live.
    pub fn render(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let usable = width.saturating_sub(PREFIX_FIRST.chars().count()).max(8);
        let mut lines: Vec<String> = vec![String::new()];
        let mut cur = (0usize, 0usize);
        for (idx, &c) in self.buf.iter().enumerate() {
            if idx == self.cursor {
                cur = (lines.len() - 1, lines.last().map(|l| l.chars().count()).unwrap_or(0));
            }
            if c == '\n' {
                lines.push(String::new());
                continue;
            }
            if lines.last().map(|l| l.chars().count()).unwrap_or(0) >= usable {
                lines.push(String::new());
            }
            lines.last_mut().unwrap().push(c);
        }
        if self.cursor >= self.buf.len() {
            let last = lines.len() - 1;
            cur = (last, lines[last].chars().count());
        }
        (lines, cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(input: &mut Input, s: &str) {
        input.insert_str(s);
    }

    #[test]
    fn insert_edit_backspace() {
        let mut i = Input::new();
        typed(&mut i, "hello world");
        i.left();
        i.left();
        i.backspace(); // deletes 'r'
        assert_eq!(i.text(), "hello wold");
        i.delete(); // deletes the 'l' now sitting after the cursor
        assert_eq!(i.text(), "hello wod");
        i.home();
        assert_eq!(i.cursor_char(), 0);
        i.end();
        assert_eq!(i.cursor_char(), i.text().chars().count());
    }

    #[test]
    fn word_and_line_deletes() {
        let mut i = Input::new();
        typed(&mut i, "one two three");
        i.delete_word_left();
        assert_eq!(i.text(), "one two ");
        i.delete_word_left();
        assert_eq!(i.text(), "one ");
        typed(&mut i, "\nx");
        i.clear_to_line_start();
        assert_eq!(i.text(), "one \n");
    }

    #[test]
    fn history_browse_and_draft_restore() {
        let mut i = Input::new();
        typed(&mut i, "first");
        i.submit();
        typed(&mut i, "second");
        i.submit();
        assert!(i.history_prev());
        assert_eq!(i.text(), "second");
        assert!(i.history_prev());
        assert_eq!(i.text(), "first");
        assert!(!i.history_prev());
        assert!(i.history_next());
        assert_eq!(i.text(), "second");
        assert!(i.history_next());
        assert_eq!(i.text(), "");
        // Draft restore: typing then browsing away and back.
        typed(&mut i, "draft");
        assert!(i.history_prev());
        assert_eq!(i.text(), "second");
        assert!(i.history_next());
        assert_eq!(i.text(), "draft");
    }

    #[test]
    fn history_blocked_when_multiline() {
        let mut i = Input::new();
        typed(&mut i, "saved");
        i.submit();
        typed(&mut i, "line one");
        i.newline();
        typed(&mut i, "line two");
        assert!(!i.is_single_line());
        assert!(!i.history_prev());
    }

    #[test]
    fn submit_trims_trailing_newlines_and_dedupes() {
        let mut i = Input::new();
        typed(&mut i, "hey");
        i.newline();
        i.newline();
        assert_eq!(i.submit().unwrap(), "hey");
        typed(&mut i, "hey");
        i.submit();
        typed(&mut i, "hey");
        i.submit();
        assert_eq!(i.history.len(), 1);
        i.submit(); // blank
        assert!(i.is_empty());
    }

    #[test]
    fn render_wraps_and_tracks_cursor() {
        let mut i = Input::new();
        typed(&mut i, "abcdefghij");
        let (lines, (row, _col)) = i.render(8); // usable = 6
        assert!(lines.len() >= 2, "{lines:?}");
        assert_eq!(row, lines.len() - 1);
        let total: usize = lines.iter().map(|l| l.chars().count()).sum();
        assert_eq!(total, 10);
        // Cursor mid-word.
        for _ in 0..4 {
            i.left();
        }
        let (_, (row2, col2)) = i.render(8);
        assert!(row2 <= row);
        let _ = col2;
    }

    #[test]
    fn render_multiline_input() {
        let mut i = Input::new();
        typed(&mut i, "ab");
        i.newline();
        typed(&mut i, "cd");
        let (lines, _) = i.render(40);
        assert_eq!(lines, vec!["ab", "cd"]);
    }

    #[test]
    fn persistent_history_round_trip() {
        let mut i = Input::new();
        i.load_history("a\nb\nc\n");
        assert_eq!(i.history.len(), 3);
        assert!(i.history_prev());
        assert_eq!(i.text(), "c");
        assert_eq!(i.history_text(), "a\nb\nc\n");
    }
}
