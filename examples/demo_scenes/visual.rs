use crate::*;

/// A muted caption line, reused by several scenes below.
fn caption(text: &str, theme: &Theme) -> Element {
    element(Text::new(vec![Line::from(Span::styled(
        text.to_string(),
        theme.muted_style(),
    ))]))
}

pub(crate) fn scene_diff(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let old = "fn add(a: i32, b: i32) -> i32 {\n    a - b\n}\n\nlet total = add(2, 3);";
    let new = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nlet total = add(2, 3);\nprintln!(\"{total}\");";
    view! {
        col(gap = 1) {
            fixed(1) { node(caption("unified · red = removed, green = added", theme)) }
            grow(1) { node(Diff::new(old, new).line_numbers(true)) }
        }
    }
}

pub(crate) fn scene_ascii_font(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    view! {
        col(gap = 1) {
            fixed(5) { node(AsciiFont::new("TUIKA")) }
            fixed(1) { node(caption("embedded 5-row block font · A-Z 0-9 punctuation", theme)) }
        }
    }
}

pub(crate) fn scene_qr(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    // Byte-mode v1-4 encoder; a short URL fits comfortably in version 1-2.
    const PAYLOAD: &str = "https://everruns.com";
    let qr = QrCode::encode(PAYLOAD, QrEcc::Medium)
        .map(element)
        .unwrap_or_else(|| caption("(payload too large)", theme));
    view! {
        col(gap = 1) {
            grow(1) { node(qr) }
            fixed(1) { node(caption(&format!("QrCode::encode(\"{PAYLOAD}\", QrEcc::Medium)"), theme)) }
        }
    }
}

pub(crate) fn scene_tab_select(frame: u64, theme: &Theme) -> Element {
    let labels: Vec<Line<'static>> = ["Low", "Medium", "High", "Ultra"]
        .iter()
        .map(|s| Line::from(Span::styled((*s).to_string(), theme.text_style())))
        .collect();
    let mut state = TabSelectState::default();
    let target = (frame / 18) % labels.len() as u64;
    let right = Event::Key(Key::new(KeyCode::Right));
    for _ in 0..target {
        let _ = state.handle(&right, labels.len());
    }
    view! {
        col(gap = 1) {
            fixed(1) { node(TabSelect::new(labels, &state)) }
            fixed(1) { node(caption("← → move the selection · enter/space activates", theme)) }
        }
    }
}

pub(crate) fn scene_slider(frame: u64, theme: &Theme) -> Element {
    let swept = tuika::anim::ping_pong(frame, 160);
    let mut volume = SliderState::new(0.0, 100.0, 0.0);
    volume.set_ratio(swept);
    let contrast = SliderState::new(0.0, 10.0, 6.0);
    let mut balance = SliderState::new(0.0, 1.0, 0.0);
    balance.set_ratio(1.0 - swept);

    let row = |label: &str, s: &SliderState, theme: &Theme| -> Element {
        let lbl = Text::new(vec![Line::from(Span::styled(
            label.to_string(),
            theme.muted_style(),
        ))]);
        let slider = Slider::new(s).label(s);
        view! {
            row(gap = 1) {
                fixed(9) { node(lbl) }
                grow(1) { node(slider) }
            }
        }
    };
    view! {
        col(gap = 1) {
            fixed(1) { node(row("volume", &volume, theme)) }
            fixed(1) { node(row("contrast", &contrast, theme)) }
            fixed(1) { node(row("balance", &balance, theme)) }
        }
    }
}

pub(crate) fn scene_timeline(frame: u64, theme: &Theme) -> Element {
    // Three tracks over a shared 120-frame loop: a linear ramp, an eased ramp,
    // and a 0→1→0 pulse — each a pure function of the frame counter.
    let linear = Timeline::new()
        .keyframe(0, 0.0)
        .keyframe(120, 1.0)
        .repeat(Repeat::Loop);
    let eased = Timeline::new()
        .keyframe(0, 0.0)
        .ease(120, 1.0, tuika::anim::ease_in_out)
        .repeat(Repeat::Loop);
    let pulse = Timeline::new()
        .keyframe(0, 0.0)
        .keyframe(60, 1.0)
        .keyframe(120, 0.0)
        .repeat(Repeat::Loop);

    let track = |label: &str, value: f32, theme: &Theme| -> Element {
        let lbl = Text::new(vec![Line::from(Span::styled(
            label.to_string(),
            theme.muted_style(),
        ))]);
        view! {
            row(gap = 1) {
                fixed(11) { node(lbl) }
                grow(1) { node(ProgressBar::determinate(value).percent(true)) }
            }
        }
    };
    view! {
        col(gap = 1) {
            fixed(1) { node(track("linear", linear.sample(frame), theme)) }
            fixed(1) { node(track("ease_in_out", eased.sample(frame), theme)) }
            fixed(1) { node(track("pulse", pulse.sample(frame), theme)) }
            fixed(1) { node(caption("Timeline::new().keyframe(..).ease(..).repeat(Loop)", theme)) }
        }
    }
}

pub(crate) fn scene_toast(frame: u64, theme: &Theme) -> Element {
    let _ = (frame, theme);
    // A representative stack, newest on top. The host ticks these down; here we
    // show the steady-state look of all four levels at once.
    let mut toasts = Toasts::new(4);
    toasts.push(ToastLevel::Info, "Build started");
    toasts.push(ToastLevel::Success, "42 tests passed");
    toasts.push(ToastLevel::Warning, "2 unused imports");
    toasts.push(ToastLevel::Error, "Deploy to prod failed");
    let toasts: &'static Toasts = Box::leak(Box::new(toasts));
    view! {
        col {
            grow(1) { node(ToastList::new(toasts)) }
        }
    }
}

pub(crate) fn scene_console(frame: u64, theme: &Theme) -> Element {
    let _ = (frame, theme);
    let log = ConsoleLog::new(50);
    for line in [
        "INFO  server listening on 127.0.0.1:8080",
        "DEBUG loaded 12 routes in 3ms",
        "INFO  GET /  200  1.2ms",
        "WARN  slow query: users.by_email  214ms",
        "INFO  GET /health  200  0.3ms",
        "ERROR upstream timeout after 5s",
        "INFO  reconnecting to upstream…",
    ] {
        log.line(line);
    }
    let log: &'static ConsoleLog = Box::leak(Box::new(log));
    view! {
        col {
            grow(1) { node(Console::new(log).title(" console ")) }
        }
    }
}

pub(crate) fn scene_framebuffer(frame: u64, theme: &Theme) -> Element {
    let _ = theme;
    let (w, h) = (56u32, 22u32);
    let mut fb = FrameBuffer::new(w, h);
    // A diagonal gradient background.
    fb.shade(|x, y, _| {
        let r = 30 + (x * 120 / w) as u8;
        let g = 20;
        let b = 60 + (y * 120 / h) as u8;
        [r, g, b, 255]
    });
    // A "ball" bouncing over the canvas, drawn as an opaque square sprite.
    let bx = (tuika::anim::ping_pong(frame, 120) * (w - 8) as f32) as u32;
    let by = (tuika::anim::ping_pong(frame.wrapping_add(37), 84) * (h - 8) as f32) as u32;
    fb.fill_rect(bx, by, 8, 8, [240, 210, 90, 255]);
    // A scanline shader post-pass darkens every other row.
    fb.shade(|_x, y, [r, g, b, a]| {
        if y % 2 == 0 {
            [r, g, b, a]
        } else {
            [r / 2, g / 2, b / 2, a]
        }
    });
    let fb: &'static FrameBuffer = Box::leak(Box::new(fb));
    view! {
        col(gap = 1) {
            grow(1) { node(FrameBufferView::new(fb, w as u16, (h / 2) as u16)) }
            fixed(1) { node(caption("FrameBuffer → half-block cells · sprite + scanline shader", theme)) }
        }
    }
}
