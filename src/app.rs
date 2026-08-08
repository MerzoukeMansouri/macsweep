use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Instant;

use sysinfo::System;

use crate::clean::{self, Progress};
use crate::mem::{self, MemStats};
use crate::scan::{self, CategoryEntry};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Junk,
    Memory,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Main,
}

pub enum JunkState {
    Blank,
    Scanning(Receiver<Vec<CategoryEntry>>),
    Review {
        entries: Vec<CategoryEntry>,
        cursor: usize,
        confirm: bool,
    },
    Cleaning {
        rx: Receiver<Progress>,
        current: String,
        done_bytes: u64,
        total_bytes: u64,
    },
    Summary {
        freed: u64,
        per_category: Vec<(&'static str, u64)>,
    },
}

pub struct MemState {
    sys: System,
    pub stats: MemStats,
    last_sample: Instant,
    pub status: Option<String>,
    freeing: Option<Receiver<anyhow::Result<()>>>,
}

impl MemState {
    fn new() -> Self {
        let mut sys = System::new_all();
        let stats = mem::sample(&mut sys);
        Self {
            sys,
            stats,
            last_sample: Instant::now(),
            status: None,
            freeing: None,
        }
    }

    fn tick(&mut self) {
        if self.last_sample.elapsed().as_secs() >= 1 {
            self.stats = mem::sample(&mut self.sys);
            self.last_sample = Instant::now();
        }
        if let Some(rx) = &self.freeing {
            if let Ok(result) = rx.try_recv() {
                self.status = Some(match result {
                    Ok(()) => "Memory freed.".to_string(),
                    Err(e) => format!("purge failed: {e}"),
                });
                self.freeing = None;
            }
        }
    }

    fn free_up(&mut self) {
        if self.freeing.is_some() {
            return;
        }
        self.status = Some("Freeing memory (sudo password may be required in this terminal)...".to_string());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(mem::free_up());
        });
        self.freeing = Some(rx);
    }
}

pub struct App {
    pub panel: Panel,
    pub focus: Focus,
    pub junk: JunkState,
    pub mem: MemState,
    pub status: String,
    pub dry_run: bool,
    pub should_quit: bool,
}

impl App {
    pub fn new(dry_run: bool) -> Self {
        Self {
            panel: Panel::Junk,
            focus: Focus::Sidebar,
            junk: JunkState::Blank,
            mem: MemState::new(),
            status: "Tab: switch focus  ·  [s] scan  ·  [q] quit".to_string(),
            dry_run,
            should_quit: false,
        }
    }

    pub fn tick(&mut self) {
        self.mem.tick();

        if let JunkState::Scanning(rx) = &self.junk {
            if let Ok(entries) = rx.try_recv() {
                self.junk = JunkState::Review {
                    entries,
                    cursor: 0,
                    confirm: false,
                };
                self.status = "space: toggle  ·  [c] clean  ·  Tab: sidebar".to_string();
            }
        }

        // Drain everything pending, not just one message — the worker thread
        // finishes independently of the UI's tick rate, so a single try_recv()
        // per frame would make the display lag behind already-completed work
        // whenever there are many small items.
        let mut done = None;
        if let JunkState::Cleaning {
            rx,
            current,
            done_bytes,
            total_bytes,
        } = &mut self.junk
        {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    Progress::Item {
                        path,
                        done_bytes: db,
                        total_bytes: tb,
                    } => {
                        *current = path;
                        *done_bytes = db;
                        *total_bytes = tb;
                    }
                    Progress::Done {
                        per_category,
                        total_freed,
                    } => done = Some((per_category, total_freed)),
                }
            }
        }
        if let Some((per_category, freed)) = done {
            self.junk = JunkState::Summary { freed, per_category };
        }
    }

    fn start_scan(&mut self) {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let entries = scan::scan_all();
            let _ = tx.send(entries);
        });
        self.junk = JunkState::Scanning(rx);
        self.status = "Scanning...".to_string();
    }

    pub fn on_key(&mut self, code: crossterm::event::KeyCode) {
        use crossterm::event::KeyCode;

        if code == KeyCode::Char('q') && !matches!(self.junk, JunkState::Review { confirm: true, .. }) {
            self.should_quit = true;
            return;
        }

        if code == KeyCode::Tab {
            self.focus = match self.focus {
                Focus::Sidebar => Focus::Main,
                Focus::Main => Focus::Sidebar,
            };
            return;
        }

        if self.focus == Focus::Sidebar {
            match code {
                KeyCode::Up | KeyCode::Down => {
                    self.panel = if self.panel == Panel::Junk {
                        Panel::Memory
                    } else {
                        Panel::Junk
                    }
                }
                KeyCode::Enter | KeyCode::Right => self.focus = Focus::Main,
                _ => {}
            }
            return;
        }

        match self.panel {
            Panel::Junk => self.on_key_junk(code),
            Panel::Memory => self.on_key_memory(code),
        }
    }

    fn on_key_junk(&mut self, code: crossterm::event::KeyCode) {
        use crossterm::event::KeyCode;

        match &mut self.junk {
            JunkState::Blank => {
                if code == KeyCode::Char('s') {
                    self.start_scan();
                }
            }
            JunkState::Scanning(_) | JunkState::Cleaning { .. } => {}
            JunkState::Review {
                entries,
                cursor,
                confirm,
            } => {
                if *confirm {
                    if let KeyCode::Char('y' | 'Y') = code {
                        let selected: Vec<CategoryEntry> = entries.clone();
                        let rx = clean::spawn_delete(selected, self.dry_run);
                        self.junk = JunkState::Cleaning {
                            rx,
                            current: String::new(),
                            done_bytes: 0,
                            total_bytes: 1,
                        };
                        self.status = "Cleaning...".to_string();
                    } else {
                        *confirm = false;
                        self.status = "Cancelled.".to_string();
                    }
                    return;
                }
                match code {
                    KeyCode::Up => {
                        if *cursor > 0 {
                            *cursor -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if *cursor + 1 < entries.len() {
                            *cursor += 1;
                        }
                    }
                    KeyCode::Char(' ') => {
                        if let Some(e) = entries.get_mut(*cursor) {
                            e.selected = !e.selected;
                        }
                    }
                    KeyCode::Char('c') => {
                        let total: u64 = entries.iter().filter(|e| e.selected).map(|e| e.total_size).sum();
                        if total > 0 {
                            *confirm = true;
                            self.status = format!("Delete {}? [y/N]", crate::ui::human_size(total));
                        }
                    }
                    _ => {}
                }
            }
            JunkState::Summary { .. } => {
                self.junk = JunkState::Blank;
                self.start_scan();
            }
        }
    }

    fn on_key_memory(&mut self, code: crossterm::event::KeyCode) {
        if code == crossterm::event::KeyCode::Char('p') {
            self.mem.free_up();
        }
    }
}
