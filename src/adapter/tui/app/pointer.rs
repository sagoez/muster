use super::*;

impl App {
    /// Routes pointer input the way herdr does: sidebar rows and panes focus
    /// on click, a child that requested mouse reports receives events
    /// directly, the wheel scrolls, and a left drag selects pane text.
    pub(super) fn handle_mouse(&mut self, mouse: MouseEvent) {
        let (sidebar_area, main_area, _) = areas(self.frame_area);
        // While a toast-dismiss gesture is in flight, swallow the rest of it (the
        // drag and the release) so a mouse-reporting child never sees an
        // orphaned release from the press that closed the toast.
        if self.toast_dismiss_capture {
            if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) {
                self.toast_dismiss_capture = false;
            }
            return;
        }
        // Toasts float above everything, including a modal, so a click on one
        // closes it before the overlay swallows the event; the whole gesture is
        // consumed so it never reaches the pane beneath.
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && self.dismiss_toast_at(main_area, Position::new(mouse.column, mouse.row))
        {
            self.toast_dismiss_capture = true;
            return;
        }
        if self.overlay.is_some() {
            self.clear_selection();
            return;
        }
        self.refresh_pointer_shape(main_area, mouse);
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && sidebar_area.contains(Position::new(mouse.column, mouse.row))
        {
            self.clear_selection();
            self.handle_sidebar_press(sidebar_area, mouse);
            return;
        }
        let Some(pane) = self.selected_pane() else {
            return;
        };
        if self.pane_wants_mouse(pane) {
            self.clear_selection();
            if matches!(mouse.kind, MouseEventKind::Down(_))
                && main_area.contains(Position::new(mouse.column, mouse.row))
            {
                self.focus = Focus::Terminal;
            }
            self.forward_mouse_to_pane(pane, main_area, mouse);
            return;
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_left_press(pane, main_area, mouse);
            },
            MouseEventKind::Drag(MouseButton::Left) => {
                self.handle_left_drag(pane, main_area, mouse)
            },
            MouseEventKind::Up(MouseButton::Left) => self.handle_left_release(pane),
            MouseEventKind::ScrollUp => self.handle_wheel(pane, main_area, mouse, true),
            MouseEventKind::ScrollDown => self.handle_wheel(pane, main_area, mouse, false),
            _ => {},
        }
    }

    /// Focuses the sidebar on any click into it and, when the click lands on
    /// a row, selects that row the way keyboard navigation would.
    pub(super) fn handle_sidebar_press(&mut self, sidebar_area: Rect, mouse: MouseEvent) {
        self.focus = Focus::Sidebar;
        let (active_label, other_projects, selection) = self.sidebar_context();
        let state = sidebar::SidebarState::builder()
            .workspace(&self.workspace)
            .activity_frame(self.activity_frame)
            .focused(true)
            .active_project(&active_label)
            .other_projects(&other_projects)
            .selection(selection)
            .usage(&self.usage)
            .build();
        let Some(clicked) =
            sidebar::selection_at(&state, sidebar_area, Position::new(mouse.column, mouse.row))
        else {
            return;
        };
        match clicked {
            sidebar::SidebarSelection::Process(index) => {
                self.project_cursor = None;
                self.workspace.select_at(index);
            },
            sidebar::SidebarSelection::Project(index) => {
                self.project_cursor = Some(index);
            },
        }
    }

    /// A left press in the pane: focuses the terminal, detects a double-click
    /// word copy, and otherwise anchors a new selection at the pressed cell.
    pub(super) fn handle_left_press(&mut self, pane: PaneId, main_area: Rect, mouse: MouseEvent) {
        self.set_autoscroll(None);
        self.selection_clear_deadline = None;
        let cell = self
            .pane_grid(pane, main_area)
            .and_then(|grid| selection::cell_at(grid, mouse.column, mouse.row));
        let Some(cell) = cell else {
            self.clear_selection();
            return;
        };
        self.focus = Focus::Terminal;
        let Some(metrics) = self.pane_scroll_metrics(pane) else {
            return;
        };
        let pressed = metrics.buffer_cell(cell);
        let click = PaneClick {
            pane,
            cell: pressed,
            at: Instant::now(),
        };
        if self.take_double_click(click) && self.copy_word_at(pane, cell, pressed) {
            return;
        }
        self.selection = Some(Selection::anchored(pane, pressed));
    }

    /// Consumes a double-click candidate: true when this press matches the
    /// previous one closely enough (herdr's same-cell, 350 ms rule).
    pub(super) fn take_double_click(&mut self, click: PaneClick) -> bool {
        let matched = self.last_pane_click.is_some_and(|last| {
            last.pane == click.pane
                && last.cell == click.cell
                && click.at.duration_since(last.at) <= PANE_DOUBLE_CLICK_WINDOW
        });
        self.last_pane_click = (!matched).then_some(click);
        matched
    }

    /// Copies the token under a double-click, leaving a short-lived highlight
    /// as confirmation (herdr's word copy). Returns whether text was copied.
    pub(super) fn copy_word_at(
        &mut self,
        pane: PaneId,
        cell: GridCell,
        pressed: BufferCell,
    ) -> bool {
        let Some(target) = self.panes.get(&pane) else {
            return false;
        };
        let screen = target.parser.screen();
        let (_, columns) = screen.size();
        let row_text = screen.contents_between(cell.row(), 0, cell.row(), columns);
        let Some((start_column, end_column)) = selection::word_bounds(&row_text, cell.column())
        else {
            return false;
        };
        let text = screen.contents_between(
            cell.row(),
            start_column,
            cell.row(),
            end_column.saturating_add(1).min(columns),
        );
        if text.is_empty() {
            return false;
        }
        self.pending_clipboard = Some(text);
        self.show_toast(TOAST_COPIED, ToastTone::Success);
        let start = BufferCell::builder()
            .row(pressed.row())
            .column(start_column)
            .build();
        let end = BufferCell::builder()
            .row(pressed.row())
            .column(end_column)
            .build();
        self.selection = Some(Selection::word(pane, start, end));
        self.selection_clear_deadline = Some(Instant::now() + PANE_COPY_HIGHLIGHT_DURATION);
        true
    }

    /// Extends the held selection toward the pointer, driving herdr's edge
    /// autoscroll zones when the drag reaches or leaves the pane vertically.
    pub(super) fn handle_left_drag(&mut self, pane: PaneId, main_area: Rect, mouse: MouseEvent) {
        self.last_pane_click = None;
        let holding = self
            .selection
            .as_ref()
            .is_some_and(|active| active.pane() == pane && active.is_in_progress());
        if !holding {
            return;
        }
        let Some(grid) = self.pane_grid(pane, main_area) else {
            return;
        };
        self.extend_selection_to(pane, grid, mouse.column, mouse.row);
        if !self.selection.as_ref().is_some_and(Selection::is_dragging) {
            self.set_autoscroll(None);
            return;
        }
        let top = grid.y;
        let bottom = grid.y + grid.height - 1;
        if mouse.row < top {
            self.scroll_by(pane, selection::edge_scroll_lines(top - mouse.row), true);
            self.extend_selection_to(pane, grid, mouse.column, mouse.row);
            self.set_autoscroll(Some(Self::autoscroll_at(AutoscrollDirection::Up, mouse)));
        } else if mouse.row > bottom {
            self.scroll_by(
                pane,
                selection::edge_scroll_lines(mouse.row - bottom),
                false,
            );
            self.extend_selection_to(pane, grid, mouse.column, mouse.row);
            self.set_autoscroll(Some(Self::autoscroll_at(AutoscrollDirection::Down, mouse)));
        } else if mouse.row == top {
            self.set_autoscroll(Some(Self::autoscroll_at(AutoscrollDirection::Up, mouse)));
        } else if mouse.row == bottom {
            self.set_autoscroll(Some(Self::autoscroll_at(AutoscrollDirection::Down, mouse)));
        } else {
            self.set_autoscroll(None);
        }
    }

    /// An autoscroll record for the pointer's current position.
    pub(super) fn autoscroll_at(direction: AutoscrollDirection, mouse: MouseEvent) -> Autoscroll {
        Autoscroll::builder()
            .direction(direction)
            .column(mouse.column)
            .row(mouse.row)
            .build()
    }

    /// Completes the held gesture: a bare click clears, a finalized word copy
    /// keeps its feedback highlight, and a drag copies the spanned text and
    /// clears (herdr's copy-on-select).
    pub(super) fn handle_left_release(&mut self, pane: PaneId) {
        self.set_autoscroll(None);
        let Some(active) = self.selection else {
            return;
        };
        if active.pane() != pane || active.is_click() {
            self.selection = None;
            return;
        }
        if active.is_dragging() {
            let text = self.extract_selection_text(pane, &active);
            if let Some(text) = text.filter(|text| !text.is_empty()) {
                self.pending_clipboard = Some(text);
                self.show_toast(TOAST_COPIED, ToastTone::Success);
            }
            self.selection = None;
        }
    }

    /// The wheel scrolls the pane: an in-progress selection keeps extending
    /// under it (herdr), an alternate-screen child receives cursor keys, and
    /// otherwise the scrollback offset moves.
    pub(super) fn handle_wheel(
        &mut self,
        pane: PaneId,
        main_area: Rect,
        mouse: MouseEvent,
        up: bool,
    ) {
        if !main_area.contains(Position::new(mouse.column, mouse.row)) {
            return;
        }
        let selecting = self
            .selection
            .as_ref()
            .is_some_and(|active| active.pane() == pane && active.is_in_progress());
        if !selecting {
            let Some(target) = self.panes.get_mut(&pane) else {
                return;
            };
            let screen = target.parser.screen();
            if screen.alternate_screen() {
                let bytes = mouse::wheel_arrow(up, screen.application_cursor());
                if let Some(handle) = target.handle.as_mut() {
                    for _ in 0..WHEEL_SCROLL_LINES {
                        let _ = handle.write_input(bytes);
                    }
                }
                return;
            }
        }
        self.scroll_by(pane, WHEEL_SCROLL_LINES, up);
        if selecting && let Some(grid) = self.pane_grid(pane, main_area) {
            self.extend_selection_to(pane, grid, mouse.column, mouse.row);
        }
    }

    /// Moves the selection head to the pane cell nearest the pointer.
    pub(super) fn extend_selection_to(&mut self, pane: PaneId, grid: Rect, column: u16, row: u16) {
        let Some(cell) = selection::nearest_cell(grid, column, row) else {
            return;
        };
        let Some(metrics) = self.pane_scroll_metrics(pane) else {
            return;
        };
        if let Some(active) = self.selection.as_mut() {
            active.extend_to(metrics.buffer_cell(cell));
        }
    }

    /// Moves the pane's scrollback offset by `lines`.
    pub(super) fn scroll_by(&mut self, pane: PaneId, lines: usize, up: bool) {
        let Some(target) = self.panes.get_mut(&pane) else {
            return;
        };
        let screen = target.parser.screen_mut();
        let offset = if up {
            screen.scrollback().saturating_add(lines)
        } else {
            screen.scrollback().saturating_sub(lines)
        };
        screen.set_scrollback(offset);
    }

    /// Replaces the autoscroll state, arming its tick when one starts and
    /// disarming it when it ends.
    pub(super) fn set_autoscroll(&mut self, autoscroll: Option<Autoscroll>) {
        match (&self.selection_autoscroll, &autoscroll) {
            (None, Some(_)) => {
                self.autoscroll_deadline = Some(Instant::now() + SELECTION_AUTOSCROLL_INTERVAL);
            },
            (_, None) => self.autoscroll_deadline = None,
            _ => {},
        }
        self.selection_autoscroll = autoscroll;
    }

    /// Drops every piece of selection state: highlight, autoscroll, feedback
    /// deadline, and the double-click candidate.
    pub(super) fn clear_selection(&mut self) {
        self.selection = None;
        self.selection_view = None;
        self.set_autoscroll(None);
        self.selection_clear_deadline = None;
        self.last_pane_click = None;
    }

    /// The next moment the selection needs servicing: an autoscroll tick or a
    /// word-highlight expiry.
    pub fn next_selection_deadline(&self) -> Option<Instant> {
        match (self.autoscroll_deadline, self.selection_clear_deadline) {
            (Some(tick), Some(clear)) => Some(tick.min(clear)),
            (tick, clear) => tick.or(clear),
        }
    }

    /// Advances selection timers: expires the word-copy highlight and steps an
    /// active edge autoscroll. Returns whether a redraw is needed.
    pub fn advance_selection(&mut self, now: Instant) -> bool {
        let mut redraw = false;
        if self
            .selection_clear_deadline
            .is_some_and(|deadline| deadline <= now)
        {
            self.selection_clear_deadline = None;
            self.selection = None;
            redraw = true;
        }
        if self
            .autoscroll_deadline
            .is_some_and(|deadline| deadline <= now)
        {
            redraw |= self.tick_autoscroll(now);
        }
        redraw
    }

    /// One autoscroll step (herdr's 30 ms cadence): scroll a line toward the
    /// drag direction, stop at the buffer edge, and re-extend the selection
    /// at the pointer's last known spot.
    pub(super) fn tick_autoscroll(&mut self, now: Instant) -> bool {
        let Some(autoscroll) = self.selection_autoscroll else {
            self.autoscroll_deadline = None;
            return false;
        };
        let Some(pane) = self
            .selection
            .as_ref()
            .filter(|active| active.is_dragging())
            .map(Selection::pane)
        else {
            self.set_autoscroll(None);
            return false;
        };
        let Some(metrics) = self.pane_scroll_metrics(pane) else {
            self.set_autoscroll(None);
            return false;
        };
        let at_edge = match autoscroll.direction() {
            AutoscrollDirection::Up => metrics.offset() >= metrics.len(),
            AutoscrollDirection::Down => metrics.offset() == 0,
        };
        if at_edge {
            self.set_autoscroll(None);
            return false;
        }
        self.scroll_by(pane, 1, autoscroll.direction() == AutoscrollDirection::Up);
        let (_, main_area, _) = areas(self.frame_area);
        if let Some(grid) = self.pane_grid(pane, main_area) {
            self.extend_selection_to(pane, grid, autoscroll.column(), autoscroll.row());
        }
        self.autoscroll_deadline = Some(now + SELECTION_AUTOSCROLL_INTERVAL);
        true
    }

    /// Reads the selected text, walking the scrollback in viewport-sized
    /// chunks when the span extends beyond the visible screen.
    pub(super) fn extract_selection_text(
        &mut self,
        pane: PaneId,
        active: &Selection,
    ) -> Option<String> {
        let target = self.panes.get_mut(&pane)?;
        let screen = target.parser.screen_mut();
        let (rows, columns) = screen.size();
        if rows == 0 || columns == 0 {
            return None;
        }
        let saved = screen.scrollback();
        screen.set_scrollback(usize::MAX);
        let len = screen.scrollback();
        let (start, end) = active.span();
        let max_row = len + usize::from(rows) - 1;
        if start.row() > max_row {
            screen.set_scrollback(saved);
            return None;
        }
        let end_row = end.row().min(max_row);
        let mut text = String::new();
        let mut chunk_top = start.row();
        loop {
            screen.set_scrollback(len.saturating_sub(chunk_top));
            let viewport_top = len - screen.scrollback();
            let last_visible = viewport_top + usize::from(rows) - 1;
            let chunk_end = end_row.min(last_visible);
            let start_column = if chunk_top == start.row() {
                start.column()
            } else {
                0
            };
            let end_column = if chunk_end == end_row {
                end.column().saturating_add(1).min(columns)
            } else {
                columns
            };
            text.push_str(&screen.contents_between(
                (chunk_top - viewport_top) as u16,
                start_column,
                (chunk_end - viewport_top) as u16,
                end_column,
            ));
            if chunk_end >= end_row {
                break;
            }
            text.push('\n');
            chunk_top = last_visible + 1;
        }
        screen.set_scrollback(saved);
        Some(text)
    }

    /// Where the pane's viewport sits in its scrollback. Learning the total
    /// briefly clamps the offset to its maximum, so this needs `&mut`.
    pub(super) fn pane_scroll_metrics(&mut self, pane: PaneId) -> Option<ScrollMetrics> {
        let target = self.panes.get_mut(&pane)?;
        let screen = target.parser.screen_mut();
        let offset = screen.scrollback();
        screen.set_scrollback(usize::MAX);
        let len = screen.scrollback();
        screen.set_scrollback(offset);
        Some(ScrollMetrics::builder().offset(offset).len(len).build())
    }

    /// Recomputes the viewport span of the active selection for the next
    /// frame, so rendering itself stays immutable.
    pub fn refresh_selection_view(&mut self) {
        self.selection_view = None;
        let Some(active) = self.selection else {
            return;
        };
        if Some(active.pane()) != self.selected_pane() {
            return;
        }
        let Some(metrics) = self.pane_scroll_metrics(active.pane()) else {
            return;
        };
        let Some(target) = self.panes.get(&active.pane()) else {
            return;
        };
        let (rows, columns) = target.parser.screen().size();
        self.selection_view = active.viewport_span(metrics.viewport_top(), rows, columns);
    }

    /// Tracks which pointer shape the hovered region wants and queues the
    /// OSC 22 update when it changes: an I-beam over selectable pane text,
    /// the regular arrow everywhere else.
    pub(super) fn refresh_pointer_shape(&mut self, main_area: Rect, mouse: MouseEvent) {
        let over_text = self.selected_pane().is_some_and(|pane| {
            !self.pane_wants_mouse(pane)
                && self
                    .pane_grid(pane, main_area)
                    .is_some_and(|grid| grid.contains(Position::new(mouse.column, mouse.row)))
        });
        let shape = if over_text {
            PointerShape::Text
        } else {
            PointerShape::Default
        };
        if shape != self.pointer_shape {
            self.pointer_shape = shape;
            self.pending_pointer_shape = Some(shape);
        }
    }

    /// Whether the pane's live child explicitly enabled xterm mouse reporting.
    pub(super) fn pane_wants_mouse(&self, pane: PaneId) -> bool {
        self.panes.get(&pane).is_some_and(|target| {
            target.handle.is_some()
                && target.parser.screen().mouse_protocol_mode() != MouseProtocolMode::None
        })
    }

    /// Forwards pointer input to the terminal that requested xterm mouse mode.
    pub(super) fn forward_mouse_to_pane(&mut self, pane: PaneId, area: Rect, mouse: MouseEvent) {
        let Some(target) = self.panes.get(&pane) else {
            return;
        };
        let screen = target.parser.screen();
        let mode = screen.mouse_protocol_mode();
        let Some((column, row)) =
            Self::relative_mouse_position(area, mouse.column, mouse.row, screen)
        else {
            return;
        };
        let bytes = mouse::encode_mouse(mouse, column, row, mode, screen.mouse_protocol_encoding());
        if let Some(bytes) = bytes
            && let Some(target) = self.panes.get_mut(&pane)
            && let Some(handle) = target.handle.as_mut()
        {
            let _ = handle.write_input(&bytes);
        }
    }

    /// Maps an outer-terminal mouse coordinate into the child terminal grid.
    pub(super) fn relative_mouse_position(
        area: Rect,
        column: u16,
        row: u16,
        screen: &Screen,
    ) -> Option<(u16, u16)> {
        let x = column.checked_sub(area.x + BORDER_THICKNESS)?;
        let y = row.checked_sub(area.y + BORDER_THICKNESS)?;
        let (rows, columns) = screen.size();
        (x < columns && y < rows).then_some((x, y))
    }

    /// The absolute rectangle of the pane's visible cells: the main area inside
    /// its border, intersected with the child screen size.
    pub(super) fn pane_grid(&self, pane: PaneId, main_area: Rect) -> Option<Rect> {
        let target = self.panes.get(&pane)?;
        let (rows, columns) = target.parser.screen().size();
        let width = main_area
            .width
            .saturating_sub(BORDER_THICKNESS * 2)
            .min(columns);
        let height = main_area
            .height
            .saturating_sub(BORDER_THICKNESS * 2)
            .min(rows);
        (width > 0 && height > 0).then(|| {
            Rect::new(
                main_area.x + BORDER_THICKNESS,
                main_area.y + BORDER_THICKNESS,
                width,
                height,
            )
        })
    }
}
