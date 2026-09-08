use crossterm::{
    event::{self, EnableMouseCapture, Event, KeyCode, MouseEvent, MouseEventKind},
    queue,
    terminal::{enable_raw_mode, EnterAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};

#[derive(Clone, PartialEq, Debug)]
enum Focus { L, C, R }

struct D { focus: Focus, l: u16, r: u16 }

impl D {
    fn new() -> Self { Self { focus: Focus::C, l: 15, r: 15 } }

    fn key(&mut self, k: KeyCode) {
        match k {
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::L => Focus::C,
                    Focus::C => Focus::R,
                    Focus::R => Focus::L,
                };
            }
            KeyCode::Left if self.focus == Focus::L && self.l > 10 => { self.l -= 1; }
            KeyCode::Right if self.focus == Focus::L && self.l < 40 => { self.l += 1; }
            KeyCode::Left if self.focus == Focus::R && self.r > 10 => { self.r -= 1; }
            KeyCode::Right if self.focus == Focus::R && self.r < 40 => { self.r += 1; }
            KeyCode::Char('q') | KeyCode::Esc => std::process::exit(0),
            _ => {}
        }
    }

    fn mouse(&mut self, e: &MouseEvent) {
        match e.kind {
            MouseEventKind::Down(_) => {
                let w = e.column;
                if w < self.l { self.focus = Focus::L; }
                else if w >= 100 - self.r { self.focus = Focus::R; }
                else { self.focus = Focus::C; }
            }
            MouseEventKind::ScrollUp => { if self.r > 10 { self.r -= 1; } }
            MouseEventKind::ScrollDown => { if self.r < 40 { self.r += 1; } }
            _ => {}
        }
    }

    fn render(&self, fr: &mut Frame) {
        let a = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(self.l),
                Constraint::Percentage(100 - self.l - self.r),
                Constraint::Percentage(self.r),
            ])
            .split(fr.size());

        let ml = self.focus == Focus::L;
        let mc = self.focus == Focus::C;
        let mr = self.focus == Focus::R;

        // 用静态字符串避免生命周期问题
        let left_text = "\u{1F4C1} TASKS\n  * default (3)\n  * bid-A (2)\n\n  [Tab]switch [mouse]click\n  [scroll]resize  [q]quit";
        let center_text = "\u{1F4C4} PREVIEW\n  Click artifact to preview\n\n  v docx/xlsx -> ASCII\n  ^ png -> char art\n  ^ pdf -> text only\n\n  [Tab]switch [mouse]click";
        let right_text = "\u{1F4E6} FILES\n  1. bid.docx  38KB\n  2. plan.png  412KB\n\n  [o] open [mouse]click";

        let mk = |content: &'static str, on: bool, title: &'static str| -> Paragraph {
            let fg = if on { Color::Green } else { Color::White };
            let mods = if on { Modifier::BOLD } else { Modifier::empty() };
            // 手动构建，第一行着色
            let first_line = content.lines().next().unwrap_or("");
            let rest_lines: Vec<&str> = content.lines().skip(1).collect();

            let mut para_text = ratatui::text::Text::default();
            para_text.lines.push(ratatui::text::Line::from(
                ratatui::text::Span::styled(first_line.to_string(), Style::default().fg(fg).add_modifier(mods))
            ));
            for line in rest_lines {
                para_text.lines.push(ratatui::text::Line::from(line.to_string()));
            }

            Paragraph::new(para_text).block(
                Block::default().title(title).borders(Borders::ALL).border_type(ratatui::widgets::BorderType::Thick)
            )
        };

        fr.render_widget(mk(left_text, ml, " LEFT "), a[0]);
        fr.render_widget(mk(center_text, mc, " CENTER"), a[1]);
        fr.render_widget(mk(right_text, mr, "RIGHT "), a[2]);

        let s = format!(" Focus:{:?} L:{}% C:{}% R:{}% [Tab]switch [mouse]click [scroll]resize [q]quit ", self.focus, self.l, 100-self.l-self.r, self.r);
        fr.render_widget(Paragraph::new(s).block(Block::default().borders(Borders::TOP).style(Style::default().fg(Color::Yellow))), fr.size());
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut st = std::io::stdout();
    queue!(st, EnterAlternateScreen, EnableMouseCapture)?;
    let be = CrosstermBackend::new(st);
    let mut t = Terminal::new(be)?;
    let mut d = D::new();
    loop {
        t.draw(|fr: &mut Frame| d.render(fr))?;
        match event::read()? {
            Event::Key(k) => d.key(k.code),
            Event::Mouse(e) => d.mouse(&e),
            Event::Resize(_, _) => continue,
            _ => {}
        }
    }
}
