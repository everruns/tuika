//! Reading laid-out rects back out of the view tree.
//!
//! tuika's solver assigns every view a screen [`Rect`], but keeps it internal —
//! views render into the rect they're handed and return nothing. A host that
//! builds an app UI still needs some of those rects *after* the layout runs: to
//! place the terminal cursor inside a composer, to bound a mouse selection to
//! the transcript, or to register a clickable region. [`RectProbe`] closes that
//! gap: wrap a view in [`Probe`] and the rect it was painted into is recorded
//! into a shared handle the host can read once the frame is painted.
//!
//! ```ignore
//! let input = RectProbe::new();
//! let root = view! {
//!     col {
//!         grow(1) { node(transcript) }
//!         fixed(3) { node(input.wrap(composer)) }   // reserve + probe the input
//!     }
//! };
//! paint(frame.buffer_mut(), area, &theme, root.as_ref(), &[]);
//! // Now `input.rect()` is where the composer landed — place the cursor in it.
//! ```
//!
//! Sometimes the rect is needed *before* the frame is painted. A split-footer
//! host has to publish the transcript rows its footer cannot show before it
//! paints the footer, because publishing scrolls the terminal and clears the
//! viewport; how many rows the footer shows is decided by the same layout the
//! paint will run. [`resolve_layout`] runs that layout without touching the
//! frame, so the host can publish against the geometry the paint will use
//! instead of re-deriving it with hand-written arithmetic.

use std::cell::Cell;
use std::rc::Rc;

use crate::geometry::Rect;

use crate::buffer::Buffer;
use crate::geometry::Size;
use crate::overlay::Overlay;
use crate::surface::Surface;
use crate::view::{Element, RenderCtx, ScopedElement, View, element};

/// A shared handle that receives the screen [`Rect`] tuika assigned to a
/// [`Probe`]-wrapped view during the last paint. Cheap to clone (shared cell);
/// reads back [`Rect::ZERO`] until the probed view has been painted at least
/// once, and updates every frame.
#[derive(Clone, Debug, Default)]
pub struct RectProbe(Rc<Cell<Rect>>);

impl RectProbe {
    /// Create a fresh handle reading back [`Rect::ZERO`] until its probed view
    /// is painted.
    pub fn new() -> Self {
        Self::default()
    }

    /// The rect the probed view was last painted into ([`Rect::ZERO`] if it has
    /// not been painted, e.g. it was laid out with zero size or clipped away).
    pub fn rect(&self) -> Rect {
        self.0.get()
    }

    /// Wrap `view` so its painted rect is reported here. Shorthand for
    /// [`Probe::new`].
    pub fn wrap<'view, V: View + 'view>(&self, view: V) -> ScopedElement<'view> {
        element(Probe::new(view, self))
    }

    fn set(&self, rect: Rect) {
        self.0.set(rect);
    }
}

/// A view that records the area it is painted into via a [`RectProbe`], then
/// renders its inner view unchanged. Layout-transparent: it measures and draws
/// exactly as the wrapped view does.
pub struct Probe<V: View = Element> {
    inner: V,
    probe: RectProbe,
}

impl<V: View> Probe<V> {
    /// Wrap `inner` so its painted rect is reported to `probe`.
    pub fn new(inner: V, probe: &RectProbe) -> Self {
        Self {
            inner,
            probe: probe.clone(),
        }
    }
}

impl<V: View> View for Probe<V> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        self.inner.measure(available, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        self.probe.set(area);
        self.inner.render(area, surface, ctx);
    }
}

/// Lay out `root` (and `overlays`) into `area` without painting the frame, so
/// every [`RectProbe`] in the tree reads back the rect a [`paint`] with the
/// same arguments would assign it.
///
/// The tree renders into a private scratch buffer that is then dropped, so the
/// host's frame is untouched. The pass is side-effect free towards the
/// terminal: views see [`RenderCtx::is_layout_pass`], so an image records no
/// placement and the real paint that follows emits each image once. Views that warm a
/// cache while rendering (a `MarkdownState`, a wrap cache) warm it here too,
/// which the paint that follows then reuses.
///
/// Use it when a decision that must happen *before* the paint depends on
/// geometry the paint decides — publishing above a split footer is the
/// motivating case. The usual shape is: build the tree, resolve it, act on the
/// probed rects, rebuild the tree with the updated state, and paint. Build fresh
/// probes for each frame: a probe whose view is not laid out keeps the rect it
/// last received.
///
/// [`paint`]: crate::paint
pub fn resolve_layout(area: Rect, ctx: &RenderCtx, root: &dyn View, overlays: &[Overlay]) {
    // Overlays may sit anywhere on screen, not only inside `area`; size the
    // scratch to cover all of them so none is clipped out of its probe.
    let bounds = overlays
        .iter()
        .fold(area, |bounds, overlay| bounds.union(overlay.area));
    let mut scratch = Buffer::empty(bounds);
    let ctx = ctx.for_layout_pass();
    crate::host::paint_with_context(&mut scratch, area, &ctx, root, overlays);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Flex, Text};
    use crate::geometry::Rect;
    use crate::style::Theme;
    use crate::view::{element, view_fn};

    #[test]
    fn probe_reports_the_rect_a_view_was_painted_into() {
        let probe = RectProbe::new();
        assert_eq!(probe.rect(), Rect::ZERO, "an unpainted probe reads ZERO");
        let root = Flex::column()
            .fixed(1, element(Text::raw("header")))
            .grow(1, probe.wrap(element(Text::raw("body"))));
        let theme = Theme::default();
        let _ = crate::testing::render(&root, 10, 5, &theme);
        // The grown body lands below the 1-row header and fills the rest.
        assert_eq!(probe.rect(), Rect::new(0, 1, 10, 4));
    }

    #[test]
    fn resolve_layout_reports_the_rects_a_paint_would() {
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let header = RectProbe::new();
        let body = RectProbe::new();
        let root = Flex::column()
            .fixed(2, header.wrap(element(Text::raw("header"))))
            .grow(1, body.wrap(element(Text::raw("body"))));
        let area = Rect::new(3, 4, 20, 9);

        resolve_layout(area, &ctx, &root, &[]);
        let resolved = (header.rect(), body.rect());
        assert_eq!(resolved, (Rect::new(3, 4, 20, 2), Rect::new(3, 6, 20, 7)));

        let mut buffer = Buffer::empty(Rect::new(0, 0, 30, 20));
        crate::paint_with_context(&mut buffer, area, &ctx, &root, &[]);
        assert_eq!((header.rect(), body.rect()), resolved);
    }

    #[test]
    fn resolve_layout_reaches_probes_inside_overlays_outside_the_area() {
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let popup = RectProbe::new();
        let menu = Flex::column().fixed(1, popup.wrap(element(Text::raw("menu"))));
        let overlay = crate::Overlay {
            area: Rect::new(10, 0, 6, 3),
            view: &menu,
            clear: true,
        };

        resolve_layout(Rect::new(0, 5, 8, 4), &ctx, &Text::raw("base"), &[overlay]);

        assert_eq!(popup.rect(), Rect::new(10, 0, 6, 1));
    }

    #[test]
    fn resolve_layout_survives_an_empty_area() {
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let body = RectProbe::new();
        let root = Flex::column().grow(1, body.wrap(element(Text::raw("body"))));

        resolve_layout(Rect::ZERO, &ctx, &root, &[]);

        assert!(body.rect().is_empty());
    }

    #[test]
    fn resolve_layout_records_no_image_placement() {
        use crate::components::Image;
        use crate::term::image::{ImageData, ImageLayer, ImageSupport};

        let theme = Theme::default();
        let context_layer = ImageLayer::new();
        let own_layer = ImageLayer::new();
        let ctx = RenderCtx::new(&theme).with_image_graphics(ImageSupport::Kitty, &context_layer);
        let pixels = ImageData::from_rgba(1, 1, vec![0, 0, 0, 255]).expect("rgba");
        let root = Flex::column()
            .fixed(2, element(Image::new(pixels.clone(), 4, 2)))
            .fixed(
                2,
                element(
                    Image::new(pixels, 4, 2)
                        .support(ImageSupport::Kitty)
                        .in_layer(&own_layer),
                ),
            );
        let area = Rect::new(0, 0, 4, 4);

        resolve_layout(area, &ctx, &root, &[]);
        assert!(context_layer.is_empty() && own_layer.is_empty());

        let mut buffer = Buffer::empty(area);
        crate::paint_with_context(&mut buffer, area, &ctx, &root, &[]);
        assert_eq!((context_layer.len(), own_layer.len()), (1, 1));
    }

    #[test]
    fn resolve_layout_records_no_screen_reader_nodes() {
        let theme = Theme::default();
        let frame = crate::access::AccessFrame::new();
        let ctx = RenderCtx::new(&theme).with_access(&frame);
        let probe = RectProbe::new();
        let root = view_fn(
            |_, _| Size::new(4, 1),
            |area, _surface: &mut Surface, ctx: &RenderCtx| {
                ctx.record_access(|| {
                    crate::access::Node::new(crate::access::Role::Text, "label").area(area)
                });
            },
        );
        let root = Flex::column().fixed(1, probe.wrap(root));
        let area = Rect::new(0, 0, 4, 1);

        resolve_layout(area, &ctx, &root, &[]);
        assert!(frame.nodes().is_empty());
        assert_eq!(probe.rect(), area);

        let mut buffer = Buffer::empty(area);
        crate::paint_with_context(&mut buffer, area, &ctx, &root, &[]);
        assert_eq!(frame.nodes().len(), 1);
    }

    /// The case `resolve_layout` exists for: a split-footer host learns how
    /// many transcript rows its footer shows *before* it paints, publishes the
    /// rest, then paints the same tree over the rows it kept.
    #[test]
    fn resolve_layout_sizes_a_split_footer_transcript_before_the_paint() {
        let theme = Theme::default();
        let ctx = RenderCtx::new(&theme);
        let transcript: Vec<String> = (1..=9).map(|n| format!("line {n}")).collect();
        let footer = |kept: &[String], probe: &RectProbe| {
            let lines: Vec<crate::ui::Line<'static>> = kept
                .iter()
                .map(|l| crate::ui::Line::from(l.clone()))
                .collect();
            Flex::column()
                .grow(
                    1,
                    probe.wrap(element(
                        Flex::column()
                            .justify(crate::Justify::End)
                            .fixed(lines.len() as u16, element(Text::new(lines))),
                    )),
                )
                .fixed(1, element(Text::raw("> composer")))
                .fixed(1, element(Text::raw("status")))
        };
        let area = Rect::new(0, 0, 12, 6);

        let probe = RectProbe::new();
        resolve_layout(area, &ctx, &footer(&transcript, &probe), &[]);
        let keep = probe.rect().height as usize;
        assert_eq!(keep, 4, "two chrome rows leave four for the transcript");
        let (published, kept) = transcript.split_at(transcript.len() - keep);
        assert_eq!(published.len(), 5);

        let probe = RectProbe::new();
        let frame = crate::testing::render(&footer(kept, &probe), 12, 6, &theme);
        assert_eq!(
            crate::testing::grid(&frame),
            "line 6      \nline 7      \nline 8      \nline 9      \n> composer  \nstatus      "
        );
    }
}
