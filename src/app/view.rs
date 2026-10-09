//! View rendering for GhostClipboard history interface.

use iced::widget::{
    Space, button, column, container, mouse_area, row, scrollable, text, text_input,
};
use iced::{Element, Length};

use crate::app::theme::{self, ACCENT_AMBER, ACCENT_BLUE, ACCENT_GREEN, ACCENT_RED, TEXT_MUTED};
use crate::app::time;
use crate::app::{EntryFilter, Message, SEARCH_INPUT_ID, State, ViewTab};
use crate::domain::entry::{ContentKind, EntrySummary};

/// Render the complete window interface.
pub fn view<'a>(state: &'a State) -> Element<'a, Message> {
    let header = render_header(state);
    let content = match state.active_tab {
        ViewTab::History => render_history_view(state),
        ViewTab::Settings => render_settings_view(state),
    };
    let status_bar = render_status_bar(state);

    container(
        column![header, content, status_bar]
            .spacing(0)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .style(theme::root_container)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Top navigation and search header.
fn render_header<'a>(state: &'a State) -> Element<'a, Message> {
    let brand = row![
        text("GhostClipboard").size(15),
        text("v0.1.0").size(11).color(TEXT_MUTED),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center);

    let search_field = text_input(
        "Search clipboard history... (↑↓ to select, Enter to copy)",
        &state.search_query,
    )
    .id(SEARCH_INPUT_ID)
    .on_input(Message::SearchInputChanged)
    .style(theme::search_input_style)
    .padding(6)
    .size(13);

    let mut search_row = row![search_field].align_y(iced::Alignment::Center);

    if !state.search_query.is_empty() {
        let clear_search_btn = button(text("✕").size(11))
            .on_press(Message::ClearSearch)
            .style(theme::subtle_button)
            .padding([2, 6]);
        search_row = search_row.push(clear_search_btn);
    }

    let search_container = container(search_row)
        .style(theme::search_bar_container)
        .padding([2, 8])
        .width(Length::Fill);

    let pause_label = if state.is_paused {
        "⏸ Paused"
    } else {
        "● Recording"
    };
    let pause_color = if state.is_paused {
        ACCENT_AMBER
    } else {
        ACCENT_GREEN
    };
    let pause_btn = button(text(pause_label).size(12).color(pause_color))
        .on_press(Message::TogglePause)
        .style(theme::subtle_button)
        .padding([4, 10]);

    let tab_history_btn = button(text("History").size(12))
        .on_press(Message::SwitchTab(ViewTab::History))
        .style(theme::pill_button(state.active_tab == ViewTab::History))
        .padding([4, 10]);

    let tab_settings_btn = button(text("Settings").size(12))
        .on_press(Message::SwitchTab(ViewTab::Settings))
        .style(theme::pill_button(state.active_tab == ViewTab::Settings))
        .padding([4, 10]);

    container(
        column![
            row![
                brand,
                Space::new().width(Length::Fill),
                pause_btn,
                tab_history_btn,
                tab_settings_btn
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            search_container,
        ]
        .spacing(8),
    )
    .padding(12)
    .style(|_t| container::Style {
        background: Some(iced::Background::Color(theme::BG_INPUT)),
        border: iced::Border {
            color: theme::BORDER_SUBTLE,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// Main clipboard history split view.
fn render_history_view<'a>(state: &'a State) -> Element<'a, Message> {
    if state.confirm_clear {
        return render_clear_confirmation();
    }

    let filter_bar = render_filter_bar(state);
    let list_pane = render_entries_list(state);
    let detail_pane = render_detail_pane(state);

    let split_content = row![
        container(list_pane)
            .width(Length::FillPortion(5))
            .height(Length::Fill),
        container(detail_pane)
            .width(Length::FillPortion(6))
            .height(Length::Fill)
            .padding(10)
            .style(theme::detail_pane),
    ]
    .spacing(8)
    .width(Length::Fill)
    .height(Length::Fill);

    column![filter_bar, split_content]
        .spacing(6)
        .padding(10)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Filter bar above the history list.
fn render_filter_bar<'a>(state: &'a State) -> Element<'a, Message> {
    let total_count = state.entries.len();
    let all_label = format!("All ({total_count})");

    let all_btn = button(text(all_label).size(12))
        .on_press(Message::FilterChanged(EntryFilter::All))
        .style(theme::pill_button(state.entry_filter == EntryFilter::All))
        .padding([3, 10]);

    let pinned_btn = button(text("📌 Pinned").size(12))
        .on_press(Message::FilterChanged(EntryFilter::PinnedOnly))
        .style(theme::pill_button(
            state.entry_filter == EntryFilter::PinnedOnly,
        ))
        .padding([3, 10]);

    let clear_btn = button(text("Clear History").size(12).color(ACCENT_RED))
        .on_press(Message::RequestClearHistory)
        .style(theme::subtle_button)
        .padding([3, 8]);

    row![
        all_btn,
        pinned_btn,
        Space::new().width(Length::Fill),
        clear_btn
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .into()
}

/// Confirmation banner before wiping history.
fn render_clear_confirmation<'a>() -> Element<'a, Message> {
    let prompt = text("Clear Clipboard History").size(16);
    let explanation = text(
        "Choose whether to clear only unpinned entries (retaining your saved favorites) or wipe all stored history permanently.",
    )
    .size(13)
    .color(theme::TEXT_SECONDARY);

    let clear_unpinned_btn = button(text("Clear Unpinned (Keep Favorites)").size(13))
        .on_press(Message::ClearUnpinned)
        .style(theme::primary_button)
        .padding([6, 12]);

    let clear_all_btn = button(text("Clear Everything").size(13))
        .on_press(Message::ClearAll)
        .style(theme::danger_button)
        .padding([6, 12]);

    let cancel_btn = button(text("Cancel").size(13))
        .on_press(Message::CancelClearHistory)
        .style(theme::subtle_button)
        .padding([6, 12]);

    let actions = row![clear_unpinned_btn, clear_all_btn, cancel_btn]
        .spacing(10)
        .align_y(iced::Alignment::Center);

    container(
        column![prompt, explanation, actions]
            .spacing(12)
            .padding(20),
    )
    .style(theme::detail_pane)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Left pane: list of entry summaries.
fn render_entries_list<'a>(state: &'a State) -> Element<'a, Message> {
    let filtered_indices: Vec<usize> = state
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| match state.entry_filter {
            EntryFilter::All => true,
            EntryFilter::PinnedOnly => entry.pinned,
        })
        .map(|(i, _)| i)
        .collect();

    if state.is_loading && state.entries.is_empty() {
        return container(
            text("Loading clipboard history...")
                .size(13)
                .color(TEXT_MUTED),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into();
    }

    if filtered_indices.is_empty() {
        let msg = if !state.search_query.is_empty() {
            format!("No entries matching \"{}\"", state.search_query)
        } else if state.entry_filter == EntryFilter::PinnedOnly {
            "No pinned entries. Click 📌 on any entry to pin it.".to_string()
        } else {
            "Clipboard history is empty.\nCopy any text (Ctrl+C) to automatically capture it here."
                .to_string()
        };

        return container(text(msg).size(13).color(TEXT_MUTED))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .padding(16)
            .into();
    }

    let mut list_col = column![].spacing(6);

    for &index in &filtered_indices {
        let entry = &state.entries[index];
        let is_selected = state.selected_index == Some(index);
        let item_card = render_entry_card(entry, index, is_selected);
        list_col = list_col.push(item_card);
    }

    scrollable(list_col)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// A single entry row in the list.
fn render_entry_card<'a>(
    entry: &'a EntrySummary,
    index: usize,
    is_selected: bool,
) -> Element<'a, Message> {
    let mut tags_row = row![].spacing(4).align_y(iced::Alignment::Center);

    if entry.pinned {
        tags_row = tags_row.push(
            container(text("PIN").size(9).color(theme::BG_ROOT))
                .style(theme::badge(ACCENT_AMBER, theme::BG_ROOT))
                .padding([1, 4]),
        );
    }

    if entry.kind == ContentKind::Url {
        tags_row = tags_row.push(
            container(text("URL").size(9).color(theme::BG_ROOT))
                .style(theme::badge(ACCENT_BLUE, theme::BG_ROOT))
                .padding([1, 4]),
        );
    }

    if let Some(ref app) = entry.source_app {
        tags_row = tags_row.push(
            container(text(app.as_str()).size(10).color(theme::TEXT_SECONDARY))
                .style(theme::badge(theme::BG_INPUT, theme::TEXT_SECONDARY))
                .padding([1, 4]),
        );
    }

    let rel_time = time::format_entry_time(entry.copied_at);
    tags_row = tags_row.push(Space::new().width(Length::Fill));
    tags_row = tags_row.push(text(rel_time).size(10).color(TEXT_MUTED));

    // Preview snippet: flatten newlines for a compact card display
    let snippet: String = entry
        .preview
        .lines()
        .take(2)
        .collect::<Vec<&str>>()
        .join(" ↵ ");
    let preview_text = text(snippet).size(12).font(iced::font::Font::MONOSPACE);

    let char_count = time::format_char_count(entry.char_count);
    let card_content = column![
        tags_row,
        preview_text,
        text(char_count).size(10).color(TEXT_MUTED),
    ]
    .spacing(4);

    let entry_id = entry.id;
    let card_container = container(card_content)
        .style(theme::entry_item(is_selected))
        .padding(8)
        .width(Length::Fill);

    mouse_area(card_container)
        .on_press(Message::SelectEntry(index))
        .on_right_press(Message::CopyEntry(entry_id))
        .into()
}

/// Right pane: detailed preview of the currently selected entry.
fn render_detail_pane<'a>(state: &'a State) -> Element<'a, Message> {
    let Some(index) = state.selected_index else {
        return container(
            text("Select an entry to view full text")
                .size(13)
                .color(TEXT_MUTED),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into();
    };

    let Some(entry) = state.entries.get(index) else {
        return container(text("No entry selected").size(13).color(TEXT_MUTED))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
    };

    let entry_id = entry.id;

    let pin_label = if entry.pinned {
        "📌 Unpin (Ctrl+P)"
    } else {
        "📌 Pin (Ctrl+P)"
    };

    let action_bar = row![
        button(text("📋 Copy to Clipboard (Enter)").size(12))
            .on_press(Message::CopyEntry(entry_id))
            .style(theme::primary_button)
            .padding([4, 10]),
        button(text(pin_label).size(12))
            .on_press(Message::TogglePin(entry_id))
            .style(theme::subtle_button)
            .padding([4, 10]),
        button(text("🗑 Delete (Del)").size(12))
            .on_press(Message::DeleteEntry(entry_id))
            .style(theme::danger_button)
            .padding([4, 8]),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let rel_time = time::format_entry_time(entry.copied_at);
    let source_str = entry.source_app.as_deref().unwrap_or("Unknown application");
    let kind_str = match entry.kind {
        ContentKind::Text => "Plain Text",
        ContentKind::Url => "Web URL",
    };

    let meta_info = column![
        row![
            text(format!("Source: {source_str}"))
                .size(11)
                .color(theme::TEXT_SECONDARY),
            Space::new().width(Length::Fill),
            text(format!("Copied: {rel_time}"))
                .size(11)
                .color(theme::TEXT_SECONDARY),
        ],
        row![
            text(format!("Format: {kind_str}"))
                .size(11)
                .color(theme::TEXT_SECONDARY),
            Space::new().width(Length::Fill),
            text(format!("Length: {} characters", entry.char_count))
                .size(11)
                .color(theme::TEXT_SECONDARY),
        ],
    ]
    .spacing(2);

    let display_text = state
        .selected_full_text
        .as_deref()
        .unwrap_or(entry.preview.as_str());

    let text_display = scrollable(
        text(display_text)
            .size(12)
            .font(iced::font::Font::MONOSPACE)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill);

    let preview_box = container(text_display)
        .style(|_t| container::Style {
            background: Some(iced::Background::Color(theme::BG_INPUT)),
            border: iced::Border {
                color: theme::BORDER_SUBTLE,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        })
        .padding(8)
        .width(Length::Fill)
        .height(Length::Fill);

    column![action_bar, meta_info, preview_box]
        .spacing(8)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Settings panel view.
fn render_settings_view<'a>(state: &'a State) -> Element<'a, Message> {
    let title = text("GhostClipboard Settings & System Health").size(16);

    let storage_section = column![
        text("Storage & Database").size(14),
        text(format!("Database file: {}", state.db_path))
            .size(12)
            .color(theme::TEXT_SECONDARY),
        text(format!(
            "Total entries recorded: {} ({} pinned)",
            state.entries_recorded, state.pinned_count
        ))
        .size(12)
        .color(theme::TEXT_SECONDARY),
    ]
    .spacing(4);

    let capture_section = column![
        text("Clipboard Monitoring").size(14),
        text(if state.is_paused {
            "Capture status: PAUSED (notifications ignored)"
        } else {
            "Capture status: ACTIVE (recording clipboard events)"
        })
        .size(12)
        .color(if state.is_paused {
            ACCENT_AMBER
        } else {
            ACCENT_GREEN
        }),
        text(format!(
            "Windows clipboard notifications received: {}",
            state.clipboard_notifications
        ))
        .size(12)
        .color(theme::TEXT_SECONDARY),
        text(format!(
            "Global shortcut activations: {}",
            state.hotkey_presses
        ))
        .size(12)
        .color(theme::TEXT_SECONDARY),
    ]
    .spacing(4);

    let shortcuts_section = column![
        text("Keyboard Shortcuts").size(14),
        text("Global Hotkey to toggle history: Ctrl+Shift+Space")
            .size(12)
            .color(theme::TEXT_SECONDARY),
        text("Arrow Up / Down: Select previous / next entry")
            .size(12)
            .color(theme::TEXT_SECONDARY),
        text("Enter: Copy selected entry to clipboard and restore previous app")
            .size(12)
            .color(theme::TEXT_SECONDARY),
        text("Escape: Dismiss history window / clear search")
            .size(12)
            .color(theme::TEXT_SECONDARY),
        text("Ctrl+P: Pin / unpin selected entry")
            .size(12)
            .color(theme::TEXT_SECONDARY),
        text("Delete (or Ctrl+D): Delete selected entry")
            .size(12)
            .color(theme::TEXT_SECONDARY),
    ]
    .spacing(4);

    let autostart_label = if state.autostart_enabled {
        "Status: ENABLED (starts automatically on Windows sign-in with --background)"
    } else {
        "Status: DISABLED (manual start only)"
    };
    let autostart_btn_text = if state.autostart_enabled {
        "Disable Launch at Login"
    } else {
        "Enable Launch at Login"
    };
    let autostart_btn = button(text(autostart_btn_text).size(12))
        .on_press(Message::ToggleAutostart)
        .style(theme::subtle_button)
        .padding([4, 10]);

    let startup_section = column![
        text("System Startup & Background Operation").size(14),
        text("GhostClipboard runs silently in the system tray when the window is closed.")
            .size(12)
            .color(theme::TEXT_SECONDARY),
        row![
            text(autostart_label)
                .size(12)
                .color(if state.autostart_enabled {
                    ACCENT_GREEN
                } else {
                    theme::TEXT_SECONDARY
                }),
            Space::new().width(Length::Fill),
            autostart_btn,
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        text("Single-instance protection: Active (duplicate launches activate this window)")
            .size(11)
            .color(TEXT_MUTED),
    ]
    .spacing(6);

    let mut content = column![
        title,
        storage_section,
        capture_section,
        startup_section,
        shortcuts_section
    ]
    .spacing(16);

    if !state.faults.is_empty() {
        let mut fault_col =
            column![text("Native Component Faults").size(14).color(ACCENT_RED)].spacing(4);
        for (component, message) in &state.faults {
            fault_col = fault_col.push(
                text(format!("{component:?}: {message}"))
                    .size(12)
                    .color(ACCENT_RED),
            );
        }
        content = content.push(fault_col);
    }

    container(scrollable(content).width(Length::Fill).height(Length::Fill))
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Bottom status bar.
fn render_status_bar<'a>(state: &'a State) -> Element<'a, Message> {
    let left_content: Element<'a, Message> = if let Some((ref msg, is_err)) = state.status_feedback
    {
        let color = if is_err { ACCENT_RED } else { ACCENT_GREEN };
        text(msg.as_str()).size(11).color(color).into()
    } else {
        text(format!(
            "{} entries ({} pinned)",
            state.entries_recorded, state.pinned_count
        ))
        .size(11)
        .color(TEXT_MUTED)
        .into()
    };

    let hints = text("↑↓ Select  •  Enter Copy  •  Esc Dismiss  •  Del Delete  •  Ctrl+P Pin")
        .size(11)
        .color(TEXT_MUTED);

    container(
        row![left_content, Space::new().width(Length::Fill), hints]
            .padding([4, 12])
            .align_y(iced::Alignment::Center)
            .width(Length::Fill),
    )
    .style(theme::status_bar)
    .width(Length::Fill)
    .into()
}
