//! Ratatui rendering (SPEC §5 layout). Pure view of [`crate::app::App`].

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, TranscriptEntry, CONNECT_FIELDS, CONNECT_KEY, SLASH_COMMANDS};

const SPINNER: [char; 4] = ['⠋', '⠙', '⠹', '⠸'];

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = (area.width * percent_x / 100).max(30);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect { x, y, width, height: height.min(area.height) }
}

fn entry_lines(entry: &TranscriptEntry) -> Vec<Line<'static>> {
    match entry {
        TranscriptEntry::User(text) => {
            let mut out = vec![Line::from(Span::styled(
                " User",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ))];
            for l in text.lines() {
                out.push(Line::from(vec![
                    Span::raw("  "),
                    Span::raw(l.to_string()),
                ]));
            }
            if text.lines().count() == 0 {
                out.push(Line::from(""));
            }
            out.push(Line::from(""));
            out
        }
        TranscriptEntry::Assistant(text) => {
            let mut out = vec![Line::from(Span::styled(
                " Anvil",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ))];
            for l in text.lines() {
                out.push(Line::from(vec![
                    Span::raw("  "),
                    Span::raw(l.to_string()),
                ]));
            }
            out.push(Line::from(""));
            out
        }
        TranscriptEntry::ToolCall(call) => vec![
            Line::from(vec![
                Span::styled(
                    format!("  > {} {}", call.name, call.summary),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
        ],
        TranscriptEntry::ToolResult(res) => {
            let (mark, color) = if res.success { ("✓", Color::Green) } else { ("✗", Color::Red) };
            let mut out = vec![Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    format!("{mark} {}", res.summary),
                    Style::default().fg(color),
                ),
            ])];
            // Show first 4 lines of output, dimmed.
            for l in res.output.lines().take(4) {
                out.push(Line::from(vec![
                    Span::raw("      "),
                    Span::styled(l.to_string(), Style::default().fg(Color::DarkGray)),
                ]));
            }
            out
        }
        TranscriptEntry::Error(msg) => {
            let mut out = Vec::new();
            for (i, l) in msg.lines().enumerate() {
                let text = if i == 0 { format!("Error: {l}") } else { l.to_string() };
                out.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(text, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                ]));
            }
            if out.is_empty() {
                out.push(Line::from(""));
            }
            out.push(Line::from(""));
            out
        }
        TranscriptEntry::System(msg) => {
            let mut out = Vec::new();
            for l in msg.lines() {
                out.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(l.to_string(), Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
                ]));
            }
            if out.is_empty() {
                out.push(Line::from(""));
            }
            out.push(Line::from(""));
            out
        }
    }
}

/// Render the whole TUI. Mutates `app.scroll` when following.
pub fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(6),
            Constraint::Length(1),
        ])
        .split(area);

    // ---- header ----
    let mut spinner = if app.busy() {
        format!(" {} thinking", SPINNER[app.spinner_tick % SPINNER.len()])
    } else {
        String::new()
    };
    // Live elapsed timer: proves work is ongoing (and for how long) even
    // when the provider is silent between events.
    if app.busy() {
        if let Some(secs) = app.elapsed_secs() {
            spinner.push_str(&format!(" · {secs}s"));
        }
    }
    let header = Paragraph::new(Line::from(vec![
        Span::styled(" ANVIL", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled(spinner, Style::default().fg(Color::Yellow)),
        Span::raw("  "),
        Span::styled(
            app.status.clone(),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" model: {} ", app.model))
            .title_alignment(Alignment::Right),
    );
    f.render_widget(header, chunks[0]);

    // ---- transcript ----
    let mut lines: Vec<Line<'static>> = Vec::new();
    for e in &app.transcript {
        lines.extend(entry_lines(e));
    }
    if !app.stream_buf.is_empty() || app.streaming {
        lines.push(Line::from(Span::styled(
            " Anvil",
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        )));
        let cursor = if app.streaming { "▌" } else { "" };
        for l in format!("{}{}", app.stream_buf, cursor).lines() {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::raw(l.to_string()),
            ]));
        }
        if app.stream_buf.is_empty() {
            lines.push(Line::from(format!("  {cursor}")));
        }
        lines.push(Line::from(""));
    }
    if app.show_working() {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{} working…", SPINNER[app.spinner_tick % SPINNER.len()]),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    // Pin to bottom when following (against the transcript's real height,
    // which shrinks when the debug pane is open).
    let debug_open = app.debug && chunks[1].height >= 10;
    let debug_h = if debug_open {
        (app.event_log.len() as u16 + 2).min(10).max(4)
    } else {
        0
    };
    let view_h = chunks[1]
        .height
        .saturating_sub(2 + debug_h) as usize;
    if app.follow {
        let total = lines.len();
        app.scroll = total.saturating_sub(view_h) as u16;
    }
    let transcript = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Conversation "))
        .wrap(Wrap { trim: false })
        .scroll((app.scroll, 0));

    // ---- under-the-hood event log (`/debug`) ----
    // Splits the conversation area: transcript on top, raw event feed below.
    if debug_open {
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(debug_h)])
            .split(chunks[1]);
        f.render_widget(transcript, areas[0]);
        let visible = debug_h.saturating_sub(2) as usize;
        let skip = app.event_log.len().saturating_sub(visible);
        let dlines: Vec<Line<'static>> = app
            .event_log
            .iter()
            .skip(skip)
            .map(|e| {
                Line::from(Span::styled(
                    format!("  {e}"),
                    Style::default().fg(Color::DarkGray),
                ))
            })
            .collect();
        f.render_widget(
            Paragraph::new(if dlines.is_empty() {
                vec![Line::from(Span::styled(
                    "  (no events yet — send a message)",
                    Style::default().fg(Color::DarkGray),
                ))]
            } else {
                dlines
            })
            .block(Block::default().borders(Borders::ALL).title(" Events (/debug hides) "))
            .wrap(Wrap { trim: false }),
            areas[1],
        );
    } else {
        f.render_widget(transcript, chunks[1]);
    }

    // ---- input ----
    let input = Paragraph::new(app.input.as_str())
        .block(Block::default().borders(Borders::ALL).title(
            " > Type a message… (Enter send · Shift+Enter newline) ",
        ))
        .wrap(Wrap { trim: false });
    f.render_widget(input, chunks[2]);
    // Place the terminal cursor inside the input box.
    let inner_x = chunks[2].x + 1;
    let inner_y = chunks[2].y + 1;
    let inner_w = chunks[2].width.saturating_sub(2);
    let before = &app.input[..app.cursor.min(app.input.len())];
    let mut row: u16 = 0;
    let mut col: u16 = 0;
    for ch in before.chars() {
        if ch == '\n' {
            row += 1;
            col = 0;
        } else {
            col += 1;
            if col >= inner_w {
                row += 1;
                col = 0;
            }
        }
    }
    f.set_cursor_position((inner_x + col, inner_y + row));

    // ---- `/` autocomplete palette (non-modal, floats above the input) ----
    let palette = app.slash_matches();
    if !palette.is_empty()
        && app.pending_approval.is_none()
        && app.connect_form.is_none()
        && !app.show_help
    {
        let sel = app.slash_selected.min(palette.len() - 1);
        let input_area = chunks[2];
        let room = input_area.y.saturating_sub(area.y);
        let height = ((palette.len() as u16) + 3).min(room);
        if height >= 5 {
            let visible = (height as usize).saturating_sub(3);
            let start = if sel + 1 > visible { sel + 1 - visible } else { 0 };
            let width = 50u16.min(area.width.saturating_sub(4)).max(30);
            let x = input_area.x.min(area.width.saturating_sub(width));
            let pop = Rect {
                x,
                y: input_area.y.saturating_sub(height),
                width: width.min(area.width.saturating_sub(x)),
                height,
            };
            f.render_widget(Clear, pop);
            let mut ptext = Vec::new();
            for (i, (name, desc)) in palette.iter().enumerate().skip(start).take(visible) {
                let hl = i == sel;
                ptext.push(Line::from(vec![
                    Span::styled(
                        format!("  {name:<9}"),
                        if hl {
                            Style::default()
                                .fg(Color::Black)
                                .bg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Cyan)
                        },
                    ),
                    Span::styled(
                        format!(" {desc}"),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
            ptext.push(Line::from(Span::styled(
                "  Tab complete · Enter run · Esc dismiss · ↑↓ select",
                Style::default().fg(Color::DarkGray),
            )));
            f.render_widget(
                Paragraph::new(ptext)
                    .block(Block::default().borders(Borders::ALL).title(" Commands "))
                    .wrap(Wrap { trim: false }),
                pop,
            );
        }
    }

    // ---- footer: key hints left, release version right ----
    let version_text = if app.version.is_empty() {
        String::new()
    } else {
        format!(" v{} ", app.version)
    };
    let foot_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(version_text.len() as u16)])
        .split(chunks[3]);
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" ↑↓ history ", Style::default().fg(Color::DarkGray)),
        Span::styled(" Enter send ", Style::default().fg(Color::DarkGray)),
        Span::styled(" / commands ", Style::default().fg(Color::DarkGray)),
        Span::styled(" Esc stop ", Style::default().fg(Color::DarkGray)),
        Span::styled(" Ctrl+C exit ", Style::default().fg(Color::DarkGray)),
        Span::styled(" PgUp/PgDn scroll ", Style::default().fg(Color::DarkGray)),
        Span::styled(" ? help ", Style::default().fg(Color::DarkGray)),
    ]));
    f.render_widget(footer, foot_chunks[0]);
    if !version_text.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                version_text,
                Style::default().fg(Color::DarkGray),
            )))
            .alignment(Alignment::Right),
            foot_chunks[1],
        );
    }

    // ---- approval modal (SPEC §12) ----
    if let Some(req) = &app.pending_approval {
        let modal_area = centered_rect(70, 9, area);
        f.render_widget(Clear, modal_area);
        let text = vec![
            Line::from(Span::styled(
                " ANVIL wants to execute:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(req.preview.clone(), Style::default().fg(Color::Yellow)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                " [Enter] Allow   [a] Always   [Esc] Deny ",
                Style::default().fg(Color::Cyan),
            )),
        ];
        let modal = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" Approval "))
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false });
        f.render_widget(modal, modal_area);
    } else if let Some(form) = &app.connect_form {
        // ---- `/connect` form ----
        // Same width math as `centered_rect(76, …)` so truncation matches.
        let modal_w = (area.width * 76 / 100).max(30);
        let value_w = modal_w.saturating_sub(14).min(120) as usize;
        let mut text = vec![
            Line::from(Span::styled(
                " Connect to provider",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        for (i, (label, _)) in CONNECT_FIELDS.iter().enumerate() {
            let focused = i == form.focused;
            let full = if i == CONNECT_KEY {
                App::mask_key(&form.values[i])
            } else {
                form.values[i].clone()
            };
            // Keep the value inside the modal borders (char-safe truncate).
            let shown: String = full.chars().take(value_w).collect();
            text.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{label:<8}"),
                    if focused {
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
                Span::raw(" "),
                Span::raw(shown),
            ]));
        }
        text.push(Line::from(""));
        if let Some(err) = &form.error {
            text.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(err.clone(), Style::default().fg(Color::Red)),
            ]));
        }
        text.push(Line::from(Span::styled(
            " [Tab] switch field   [Ctrl+D] fill defaults",
            Style::default().fg(Color::Cyan),
        )));
        text.push(Line::from(Span::styled(
            " [Enter] save   [Esc] cancel",
            Style::default().fg(Color::Cyan),
        )));
        let h = (text.len() as u16 + 2).min(area.height.max(1));
        let modal_area = centered_rect(76, h, area);
        f.render_widget(Clear, modal_area);
        f.render_widget(
            Paragraph::new(text)
                .block(Block::default().borders(Borders::ALL).title(" Connect "))
                .wrap(Wrap { trim: false }),
            modal_area,
        );
        // Terminal cursor inside the focused field.
        let fi = form.focused;
        let val = &form.values[fi];
        let before = val[..form.cursors[fi].min(val.len())].chars().count() as u16;
        let col = before.min(value_w as u16);
        f.set_cursor_position((
            modal_area.x.saturating_add(12).saturating_add(col),
            modal_area.y.saturating_add(3).saturating_add(fi as u16),
        ));
    } else if app.show_help {
        let mut text = vec![
            Line::from(Span::styled(
                " Slash commands",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        for (name, desc) in SLASH_COMMANDS {
            text.push(Line::from(format!("  {name:<10}{desc}")));
        }
        text.push(Line::from(""));
        text.push(Line::from("  type / for autocomplete · [Esc or ?] close"));
        let h = (text.len() as u16 + 2).min(area.height.max(1));
        let modal_area = centered_rect(70, h, area);
        f.render_widget(Clear, modal_area);
        let modal = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" Help "))
            .wrap(Wrap { trim: false });
        f.render_widget(modal, modal_area);
    }
}
