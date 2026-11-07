mod view;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use view::View;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    CanvasRenderingContext2d, HtmlCanvasElement, HtmlInputElement, HtmlSelectElement,
    HtmlTextAreaElement, PointerEvent, WheelEvent,
};

type Point = (f64, f64);
/// A single press-to-release stroke. Holds one point while the pointer is
/// down but hasn't moved yet. Once finished it is either a straight line
/// `[p0, p1]` (start, end), or — after clicking the line to curve it — a
/// cubic Bezier `[p0, p1, p2, p3]`, in order along the curve: `p0` is the
/// start anchor, `p1` and `p2` are its two control points, and `p3` is the
/// end anchor.
type Stroke = Vec<Point>;

/// Screen-space line width, in device pixels at zoom = 1.0. Because the
/// canvas context is scaled by the current zoom before strokes are drawn,
/// the ink itself scales with the drawing (like real ink on paper) rather
/// than staying a fixed width on screen.
const LINE_WIDTH: f64 = 3.0;
/// Width of the background-colored halo drawn under each stroke before its
/// ink, wider than `LINE_WIDTH` so it fully covers whatever is underneath.
/// A higher-layer stroke's halo therefore paints over (and so visually cuts
/// a gap in) any lower-layer line it crosses.
const OUTLINE_WIDTH: f64 = LINE_WIDTH + 4.0;
const DEFAULT_BACKGROUND: &str = "#1e1e1e";
const DEFAULT_INK: &str = "#f0f0f0";

/// Visual radius of a draggable Bezier control-point handle, in device
/// pixels at zoom = 1.0.
const HANDLE_RADIUS_PX: f64 = 5.0;
/// How close a press needs to land to a handle to grab it, in device
/// pixels — larger than `HANDLE_RADIUS_PX` so handles are easy to hit.
const HANDLE_HIT_RADIUS_PX: f64 = 10.0;
/// How close a press needs to land to a straight line to click-convert it
/// into a curve, in device pixels.
const LINE_HIT_TOLERANCE_PX: f64 = 8.0;
/// Maximum total pointer movement, in device pixels, for a press+release to
/// still count as a click (rather than a drag) on a line.
const CLICK_MOVE_THRESHOLD_PX: f64 = 6.0;
const HANDLE_COLOR: &str = "#5fb3ff";
const HANDLE_GUIDE_COLOR: &str = "rgba(95, 179, 255, 0.5)";

/// Length of each arrowhead wing, in the same world-scaling units as
/// `LINE_WIDTH` (so the arrowhead grows and shrinks with the drawing).
const ARROW_LENGTH: f64 = 14.0;
/// Angle, in radians, between each arrowhead wing and the line reversed —
/// i.e. half the arrowhead's total spread.
const ARROW_ANGLE: f64 = 0.4;

/// Fixed number of layers strokes can be filed under.
const NUM_LAYERS: usize = 5;

/// Default for how close, in device pixels, a new line's start or end
/// point needs to land to an existing stroke's endpoint to snap onto it.
/// Editable via the "Snap radius" control.
const DEFAULT_SNAP_RADIUS_PX: f64 = 12.0;

/// Default spacing, in world units, of the grid a new line's start or end
/// point snaps to when "Snap to grid" is on. Editable via "Grid size".
const DEFAULT_GRID_SIZE: f64 = 50.0;

struct AppState {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    dpr: f64,
    view: View,

    /// Current line (ink) color, as a CSS color string. Set by the line
    /// color picker; applies to all strokes, not just new ones.
    ink_color: String,
    /// Current canvas background color, as a CSS color string. Set by the
    /// background color picker.
    background_color: String,

    strokes: Vec<Stroke>,
    /// Which of the `NUM_LAYERS` fixed layers each stroke in `strokes`
    /// belongs to, in the same order (`stroke_layers[i]` is the layer of
    /// `strokes[i]`), set at the time the stroke was drawn.
    stroke_layers: Vec<usize>,
    /// Layer new strokes are filed under, chosen by the layer selector.
    active_layer: usize,
    /// How close, in device pixels, a new line's start or end point needs
    /// to land to an existing stroke's endpoint to snap onto it. Set by
    /// the "Snap radius" control.
    snap_radius_px: f64,
    /// Whether a new line's start or end point snaps to the nearest grid
    /// intersection when it isn't close enough to snap to an existing
    /// endpoint. Set by the "Snap to grid" checkbox.
    snap_to_grid: bool,
    /// Spacing, in world units, of the grid used when `snap_to_grid` is
    /// on. Set by the "Grid size" control.
    grid_size: f64,
    current_stroke: Option<Stroke>,
    /// Pointer id currently drawing (left mouse button, pen, or a lone touch).
    drawing_pointer_id: Option<i32>,
    /// Pointer id currently panning (middle mouse button).
    pan_pointer_id: Option<i32>,
    last_pan_pos: Point,

    /// Whether dragging one Bezier control point also moves the other one,
    /// mirrored across the curve's perpendicular bisector. Set by the
    /// "Symmetric curve" checkbox.
    symmetric: bool,
    /// (stroke index, point index — 1 or 2) of a Bezier control handle
    /// currently being dragged.
    dragging_handle: Option<(usize, usize)>,
    /// Index of a straight-line stroke the pointer went down on, pending
    /// confirmation at pointer-up that this was a click (not a drag) before
    /// converting it into a curve.
    click_candidate: Option<usize>,
    /// Screen position where the current press began, used to tell a click
    /// on `click_candidate` apart from a drag.
    press_pos: Option<Point>,

    /// Screen position of every touch pointer currently down, keyed by id.
    touch_points: HashMap<i32, Point>,
    /// Midpoint and separation of a two-finger gesture, as of the last event.
    gesture_prev: Option<(Point, f64)>,
}

impl AppState {
    fn canvas_pos(&self, event: &PointerEvent) -> Point {
        let rect = self.canvas.get_bounding_client_rect();
        (
            (event.client_x() as f64 - rect.left()) * self.dpr,
            (event.client_y() as f64 - rect.top()) * self.dpr,
        )
    }
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    let window = web_sys::window().ok_or("no global window")?;
    let document = window.document().ok_or("no document")?;
    let canvas = document
        .get_element_by_id("canvas")
        .ok_or("missing #canvas element")?
        .dyn_into::<HtmlCanvasElement>()?;
    let ctx = canvas
        .get_context("2d")?
        .ok_or("2d context unavailable")?
        .dyn_into::<CanvasRenderingContext2d>()?;
    let dpr = window.device_pixel_ratio();

    let state = Rc::new(RefCell::new(AppState {
        canvas: canvas.clone(),
        ctx,
        dpr,
        view: View::default(),
        ink_color: DEFAULT_INK.to_string(),
        background_color: DEFAULT_BACKGROUND.to_string(),
        strokes: Vec::new(),
        stroke_layers: Vec::new(),
        active_layer: 0,
        snap_radius_px: DEFAULT_SNAP_RADIUS_PX,
        snap_to_grid: false,
        grid_size: DEFAULT_GRID_SIZE,
        current_stroke: None,
        drawing_pointer_id: None,
        pan_pointer_id: None,
        last_pan_pos: (0.0, 0.0),
        symmetric: true,
        dragging_handle: None,
        click_candidate: None,
        press_pos: None,
        touch_points: HashMap::new(),
        gesture_prev: None,
    }));

    resize_canvas(&state.borrow());
    render(&state.borrow());

    install_listeners(&window, &canvas, &state)?;

    Ok(())
}

fn resize_canvas(state: &AppState) {
    let window = web_sys::window().expect("no global window");
    let width = window.inner_width().unwrap().as_f64().unwrap_or(0.0);
    let height = window.inner_height().unwrap().as_f64().unwrap_or(0.0);
    state.canvas.set_width((width * state.dpr).max(1.0) as u32);
    state
        .canvas
        .set_height((height * state.dpr).max(1.0) as u32);
}

fn render(state: &AppState) {
    let ctx = &state.ctx;
    let width = state.canvas.width() as f64;
    let height = state.canvas.height() as f64;

    ctx.set_fill_style_str(&state.background_color);
    ctx.fill_rect(0.0, 0.0, width, height);

    ctx.save();
    ctx.translate(state.view.pan_x, state.view.pan_y)
        .expect("translate");
    ctx.scale(state.view.zoom, state.view.zoom).expect("scale");

    ctx.set_line_cap("round");
    ctx.set_line_join("round");

    // Draw lowest layer first, and within each layer every stroke's
    // outline before any of their ink. That way a higher layer's outline
    // still paints over (and cuts a visual gap in) a lower layer's line,
    // but no stroke's outline ever paints over another stroke in its own
    // layer, since by the time any ink in a layer is drawn, that layer has
    // no more outlines left to paint.
    for layer in 0..NUM_LAYERS {
        let indices: Vec<usize> = (0..state.strokes.len())
            .filter(|&i| state.stroke_layers[i] == layer)
            .collect();
        for &i in &indices {
            draw_stroke_outline(ctx, &state.strokes[i], &state.background_color);
        }
        for &i in &indices {
            draw_stroke_ink(ctx, &state.strokes[i], &state.ink_color);
        }
    }
    if let Some(stroke) = &state.current_stroke {
        draw_stroke(ctx, stroke, &state.ink_color, &state.background_color);
    }
    for stroke in &state.strokes {
        if stroke.len() == 4 {
            draw_handles(ctx, stroke, state.view.zoom);
        }
    }

    ctx.restore();
}

/// Update the in-progress stroke's end point, keeping it a straight line
/// from the original press position to `world`.
fn set_stroke_end(stroke: &mut Stroke, world: Point) {
    if stroke.len() < 2 {
        stroke.push(world);
    } else {
        stroke[1] = world;
    }
}

/// Build the current path for `stroke` (a straight line or a Bezier
/// curve) plus an arrowhead at its end, without stroking it.
fn build_stroke_path(ctx: &CanvasRenderingContext2d, stroke: &Stroke) {
    // The stroke's end point and the direction it's travelling as it
    // arrives there, used to place and aim the arrowhead; `None` for an
    // in-progress stroke with no path yet.
    let end_and_tangent = match stroke.len() {
        2 => {
            ctx.begin_path();
            ctx.move_to(stroke[0].0, stroke[0].1);
            ctx.line_to(stroke[1].0, stroke[1].1);
            let dir = (stroke[1].0 - stroke[0].0, stroke[1].1 - stroke[0].1);
            Some((stroke[1], dir))
        }
        4 => {
            // stroke is [p0, p1, p2, p3], in order along the curve: p0 is
            // the start anchor, p1 and p2 are its control points, p3 is the
            // end anchor.
            ctx.begin_path();
            ctx.move_to(stroke[0].0, stroke[0].1);
            ctx.bezier_curve_to(
                stroke[1].0,
                stroke[1].1,
                stroke[2].0,
                stroke[2].1,
                stroke[3].0,
                stroke[3].1,
            );
            // The curve's tangent at its end is proportional to p3 - p2.
            let dir = (stroke[3].0 - stroke[2].0, stroke[3].1 - stroke[2].1);
            Some((stroke[3], dir))
        }
        _ => None,
    };

    if let Some((tip, dir)) = end_and_tangent {
        add_arrowhead(ctx, tip, dir);
    }
}

/// Add the two short "wing" segments of an arrowhead at `tip`, aimed back
/// along `dir` (the direction of travel arriving at `tip`). A no-op if
/// `dir` is the zero vector (a degenerate, zero-length line or curve).
fn add_arrowhead(ctx: &CanvasRenderingContext2d, tip: Point, dir: Point) {
    let len = (dir.0 * dir.0 + dir.1 * dir.1).sqrt();
    if len == 0.0 {
        return;
    }
    let back = (-dir.0 / len, -dir.1 / len);
    for &angle in &[ARROW_ANGLE, -ARROW_ANGLE] {
        let (sin, cos) = angle.sin_cos();
        let wing = (back.0 * cos - back.1 * sin, back.0 * sin + back.1 * cos);
        ctx.move_to(tip.0, tip.1);
        ctx.line_to(tip.0 + wing.0 * ARROW_LENGTH, tip.1 + wing.1 * ARROW_LENGTH);
    }
}

/// Draw `stroke`'s background-colored outline: wide enough to cover
/// whatever is underneath it, so a higher layer's outline visually cuts a
/// gap in a lower layer's line. Draw every same-layer stroke's outline
/// before any of their ink (see `render`) so same-layer strokes never cut
/// into each other this way.
fn draw_stroke_outline(ctx: &CanvasRenderingContext2d, stroke: &Stroke, outline_color: &str) {
    if stroke.len() < 2 {
        return;
    }
    ctx.set_stroke_style_str(outline_color);
    ctx.set_line_width(OUTLINE_WIDTH);
    build_stroke_path(ctx, stroke);
    ctx.stroke();
}

/// Draw `stroke`'s ink line on top of whatever outlines and ink came
/// before it.
fn draw_stroke_ink(ctx: &CanvasRenderingContext2d, stroke: &Stroke, ink_color: &str) {
    if stroke.len() < 2 {
        return;
    }
    ctx.set_stroke_style_str(ink_color);
    ctx.set_line_width(LINE_WIDTH);
    build_stroke_path(ctx, stroke);
    ctx.stroke();
}

/// Draw `stroke`'s outline immediately followed by its own ink. Used for
/// the one in-progress stroke, which isn't part of the per-layer batching
/// `render` does for finished strokes.
fn draw_stroke(
    ctx: &CanvasRenderingContext2d,
    stroke: &Stroke,
    ink_color: &str,
    outline_color: &str,
) {
    draw_stroke_outline(ctx, stroke, outline_color);
    draw_stroke_ink(ctx, stroke, ink_color);
}

/// Draw a Bezier stroke's two draggable control-point handles (`p1`, `p2`)
/// and the guide lines from each anchor to its handle.
fn draw_handles(ctx: &CanvasRenderingContext2d, stroke: &Stroke, zoom: f64) {
    let r = HANDLE_RADIUS_PX / zoom;
    ctx.save();
    ctx.set_line_width(1.0 / zoom);
    ctx.set_stroke_style_str(HANDLE_GUIDE_COLOR);

    for &(anchor, handle) in &[(stroke[0], stroke[1]), (stroke[3], stroke[2])] {
        ctx.begin_path();
        ctx.move_to(anchor.0, anchor.1);
        ctx.line_to(handle.0, handle.1);
        ctx.stroke();
    }

    ctx.set_fill_style_str(HANDLE_COLOR);
    for &(x, y) in &[stroke[1], stroke[2]] {
        ctx.begin_path();
        let _ = ctx.arc(x, y, r, 0.0, std::f64::consts::PI * 2.0);
        ctx.fill();
    }
    ctx.restore();
}

/// Default control points when a straight line first becomes a curve:
/// colinear points a third of the way in from each end, so the curve looks
/// identical to the line until the handles are dragged.
fn default_bezier_controls(p0: Point, p1: Point) -> (Point, Point) {
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    (
        (p0.0 + dx / 3.0, p0.1 + dy / 3.0),
        (p0.0 + dx * 2.0 / 3.0, p0.1 + dy * 2.0 / 3.0),
    )
}

/// Turn a straight-line stroke `[p0, p1]` into a Bezier curve `[p0, p1, p2,
/// p3]`, in order along the curve (`p0` and `p3` keep the line's original
/// start and end). A no-op if `stroke` is already a curve (or isn't a
/// finished line).
fn convert_to_bezier(stroke: &mut Stroke) {
    if stroke.len() != 2 {
        return;
    }
    let (p0, end) = (stroke[0], stroke[1]);
    let (p1, p2) = default_bezier_controls(p0, end);
    stroke[1] = p1;
    stroke.push(p2);
    stroke.push(end);
}

/// Reflect `p` across the perpendicular bisector of the segment `p0`-`p1`:
/// the tangential component (along the line) flips while the normal
/// component (perpendicular to it) is kept, so `p0` and `p1` swap places.
/// This reflection is its own inverse, which is what lets either control
/// point drive the other in symmetric mode.
fn mirror_across_perp_bisector(p0: Point, p1: Point, p: Point) -> Point {
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return p;
    }
    let mid = ((p0.0 + p1.0) / 2.0, (p0.1 + p1.1) / 2.0);
    let (vx, vy) = (p.0 - mid.0, p.1 - mid.1);
    let t = (vx * dx + vy * dy) / len2;
    (mid.0 + vx - 2.0 * t * dx, mid.1 + vy - 2.0 * t * dy)
}

/// Move Bezier control point `point_idx` (1 or 2) of `strokes[stroke_idx]`
/// to `world`. In symmetric mode the opposite control point is updated to
/// match, mirrored across the curve's perpendicular bisector.
fn update_control_point(state: &mut AppState, stroke_idx: usize, point_idx: usize, world: Point) {
    let symmetric = state.symmetric;
    if let Some(stroke) = state.strokes.get_mut(stroke_idx) {
        if stroke.len() != 4 {
            return;
        }
        stroke[point_idx] = world;
        if symmetric {
            // The curve's two anchors are p0 and p3 (start and end).
            let (p0, p3) = (stroke[0], stroke[3]);
            let mirrored = mirror_across_perp_bisector(p0, p3, world);
            let other_idx = if point_idx == 1 { 2 } else { 1 };
            stroke[other_idx] = mirrored;
        }
    }
}

/// Shortest distance from `p` to the segment `a`-`b`.
fn dist_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// Find a Bezier control handle (`p1` or `p2`) under screen position `pos`,
/// if any. Returns (stroke index, point index).
fn hit_test_handle(state: &AppState, pos: Point) -> Option<(usize, usize)> {
    for (i, stroke) in state.strokes.iter().enumerate() {
        if stroke.len() != 4 {
            continue;
        }
        for &point_idx in &[1usize, 2usize] {
            let (wx, wy) = stroke[point_idx];
            let (sx, sy) = state.view.world_to_screen(wx, wy);
            if ((pos.0 - sx).powi(2) + (pos.1 - sy).powi(2)).sqrt() <= HANDLE_HIT_RADIUS_PX {
                return Some((i, point_idx));
            }
        }
    }
    None
}

/// Find a straight-line stroke under screen position `pos`, if any. Curves
/// are excluded — only finished straight lines can be click-converted.
fn hit_test_line(state: &AppState, pos: Point) -> Option<usize> {
    state.strokes.iter().position(|stroke| {
        if stroke.len() != 2 {
            return false;
        }
        let a = state.view.world_to_screen(stroke[0].0, stroke[0].1);
        let b = state.view.world_to_screen(stroke[1].0, stroke[1].1);
        dist_to_segment(pos, a, b) <= LINE_HIT_TOLERANCE_PX
    })
}

/// Update `best` with `anchor` if it's within `snap_radius` screen pixels
/// of `pos` and closer than whatever `best` already holds.
fn consider_snap_anchor(
    view: &View,
    pos: Point,
    snap_radius: f64,
    anchor: Point,
    best: &mut Option<(f64, Point)>,
) {
    let (sx, sy) = view.world_to_screen(anchor.0, anchor.1);
    let dist = ((pos.0 - sx).powi(2) + (pos.1 - sy).powi(2)).sqrt();
    if dist <= snap_radius && best.is_none_or(|(d, _)| dist < d) {
        *best = Some((dist, anchor));
    }
}

/// Round `p` to the nearest intersection of a grid spaced `grid` world
/// units apart. A no-op (returns `p` unchanged) if `grid` isn't positive.
fn snap_to_grid_point(p: Point, grid: f64) -> Point {
    if grid <= 0.0 {
        return p;
    }
    ((p.0 / grid).round() * grid, (p.1 / grid).round() * grid)
}

/// Find the nearest existing stroke's start or end anchor within
/// `state.snap_radius_px` screen pixels of `pos`, if any.
fn find_snap_anchor(state: &AppState, pos: Point) -> Option<Point> {
    let mut best: Option<(f64, Point)> = None;
    for stroke in &state.strokes {
        match stroke.len() {
            2 => {
                consider_snap_anchor(&state.view, pos, state.snap_radius_px, stroke[0], &mut best);
                consider_snap_anchor(&state.view, pos, state.snap_radius_px, stroke[1], &mut best);
            }
            4 => {
                consider_snap_anchor(&state.view, pos, state.snap_radius_px, stroke[0], &mut best);
                consider_snap_anchor(&state.view, pos, state.snap_radius_px, stroke[3], &mut best);
            }
            _ => {}
        }
    }
    best.map(|(_, anchor)| anchor)
}

/// World position to use for a press or drag landing at screen position
/// `pos`: the nearest existing stroke's start or end anchor within
/// `state.snap_radius_px` screen pixels, so new lines snap onto it; failing
/// that, the nearest grid intersection if `state.snap_to_grid` is on;
/// otherwise `pos` converted to world space outright.
fn snap_or_world(state: &AppState, pos: Point) -> Point {
    if let Some(anchor) = find_snap_anchor(state, pos) {
        return anchor;
    }
    let world = state.view.screen_to_world(pos.0, pos.1);
    if state.snap_to_grid {
        snap_to_grid_point(world, state.grid_size)
    } else {
        world
    }
}

/// Decide what a new press at `pos` does: grab a handle, start a new
/// stroke snapped onto a nearby existing endpoint (checked before
/// click-to-convert, so a line can always be started from another line's
/// endpoint even though an endpoint is also part of that line), mark a
/// line as a click-to-convert candidate, or start an unsnapped new stroke.
fn begin_primary_press(state: &mut AppState, pos: Point) {
    if let Some(hit) = hit_test_handle(state, pos) {
        state.dragging_handle = Some(hit);
    } else if find_snap_anchor(state, pos).is_some() {
        state.current_stroke = Some(vec![snap_or_world(state, pos)]);
    } else if let Some(idx) = hit_test_line(state, pos) {
        state.click_candidate = Some(idx);
        state.press_pos = Some(pos);
    } else {
        state.current_stroke = Some(vec![snap_or_world(state, pos)]);
    }
}

/// Continue whatever `begin_primary_press` started. Returns whether a
/// redraw is needed.
fn continue_primary_press(state: &mut AppState, pos: Point) -> bool {
    if let Some((stroke_idx, point_idx)) = state.dragging_handle {
        let world = state.view.screen_to_world(pos.0, pos.1);
        update_control_point(state, stroke_idx, point_idx, world);
        true
    } else if state.click_candidate.is_some() {
        false
    } else if state.current_stroke.is_some() {
        // Snap the in-progress stroke's end point onto a nearby existing
        // endpoint, same as its start point.
        let world = snap_or_world(state, pos);
        if let Some(stroke) = state.current_stroke.as_mut() {
            set_stroke_end(stroke, world);
        }
        true
    } else {
        false
    }
}

/// Midpoint and separation distance of an exactly-two-finger touch gesture.
fn gesture_state(touch_points: &HashMap<i32, Point>) -> (Point, f64) {
    let mut positions = touch_points.values();
    let &(x0, y0) = positions.next().expect("gesture needs two touches");
    let &(x1, y1) = positions.next().expect("gesture needs two touches");
    let mid = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let dist = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    (mid, dist)
}

fn install_listeners(
    window: &web_sys::Window,
    canvas: &HtmlCanvasElement,
    state: &Rc<RefCell<AppState>>,
) -> Result<(), JsValue> {
    {
        let state = state.clone();
        let handler = Closure::<dyn FnMut(PointerEvent)>::new(move |event: PointerEvent| {
            on_pointer_down(&state, event);
        });
        canvas.add_event_listener_with_callback("pointerdown", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    {
        let state = state.clone();
        let handler = Closure::<dyn FnMut(PointerEvent)>::new(move |event: PointerEvent| {
            on_pointer_move(&state, event);
        });
        canvas.add_event_listener_with_callback("pointermove", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    {
        let state = state.clone();
        let handler = Closure::<dyn FnMut(PointerEvent)>::new(move |event: PointerEvent| {
            on_pointer_up(&state, event);
        });
        canvas.add_event_listener_with_callback("pointerup", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    {
        let state = state.clone();
        let handler = Closure::<dyn FnMut(PointerEvent)>::new(move |event: PointerEvent| {
            on_pointer_up(&state, event);
        });
        canvas
            .add_event_listener_with_callback("pointercancel", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    {
        let state = state.clone();
        let handler = Closure::<dyn FnMut(WheelEvent)>::new(move |event: WheelEvent| {
            on_wheel(&state, event);
        });
        canvas.add_event_listener_with_callback("wheel", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    if let Some(button) = window
        .document()
        .ok_or("no document")?
        .get_element_by_id("show-tikz")
    {
        let state = state.clone();
        let handler = Closure::<dyn FnMut()>::new(move || {
            let _ = show_tikz_panel(&state.borrow());
        });
        button.add_event_listener_with_callback("click", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    if let Some(button) = window
        .document()
        .ok_or("no document")?
        .get_element_by_id("close-tikz")
    {
        let handler = Closure::<dyn FnMut()>::new(move || {
            let _ = hide_tikz_panel();
        });
        button.add_event_listener_with_callback("click", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    if let Some(button) = window
        .document()
        .ok_or("no document")?
        .get_element_by_id("copy-tikz")
    {
        let handler = Closure::<dyn FnMut()>::new(move || {
            let _ = copy_tikz_to_clipboard();
        });
        button.add_event_listener_with_callback("click", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    install_checkbox_listener(window, state, "symmetric-toggle", |state, checked| {
        state.symmetric = checked;
    })?;
    install_checkbox_listener(window, state, "grid-toggle", |state, checked| {
        state.snap_to_grid = checked;
    })?;
    install_input_listener(window, state, "ink-color", |state, color| {
        state.ink_color = color;
    })?;
    install_input_listener(window, state, "background-color", |state, color| {
        state.background_color = color;
    })?;
    install_input_listener(window, state, "snap-radius", |state, value| {
        if let Ok(radius) = value.parse::<f64>() {
            state.snap_radius_px = radius.max(0.0);
        }
    })?;
    install_input_listener(window, state, "grid-size", |state, value| {
        if let Ok(size) = value.parse::<f64>() {
            state.grid_size = size.max(0.0);
        }
    })?;
    if let Some(select) = window
        .document()
        .ok_or("no document")?
        .get_element_by_id("layer-select")
    {
        let state = state.clone();
        let select = select.dyn_into::<HtmlSelectElement>()?;
        let select_el = select.clone();
        let handler = Closure::<dyn FnMut()>::new(move || {
            if let Ok(layer) = select_el.value().parse::<usize>() {
                state.borrow_mut().active_layer = layer.min(NUM_LAYERS - 1);
            }
        });
        select.add_event_listener_with_callback("change", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    {
        let state = state.clone();
        let handler = Closure::<dyn FnMut()>::new(move || {
            let state = state.borrow_mut();
            resize_canvas(&state);
            render(&state);
        });
        window.add_event_listener_with_callback("resize", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
    Ok(())
}

/// Wire any `<input>` with id `element_id` so that changing it calls
/// `apply(state, new_value)` with its string value and redraws.
fn install_input_listener(
    window: &web_sys::Window,
    state: &Rc<RefCell<AppState>>,
    element_id: &str,
    apply: impl Fn(&mut AppState, String) + 'static,
) -> Result<(), JsValue> {
    let Some(input) = window
        .document()
        .ok_or("no document")?
        .get_element_by_id(element_id)
    else {
        return Ok(());
    };
    let input = input.dyn_into::<HtmlInputElement>()?;
    let state = state.clone();
    let input_el = input.clone();
    let handler = Closure::<dyn FnMut()>::new(move || {
        let mut state = state.borrow_mut();
        apply(&mut state, input_el.value());
        render(&state);
    });
    input.add_event_listener_with_callback("input", handler.as_ref().unchecked_ref())?;
    handler.forget();
    Ok(())
}

/// Wire a `<input type="checkbox">` with id `element_id` so that changing
/// it calls `apply(state, checked)` and redraws.
fn install_checkbox_listener(
    window: &web_sys::Window,
    state: &Rc<RefCell<AppState>>,
    element_id: &str,
    apply: impl Fn(&mut AppState, bool) + 'static,
) -> Result<(), JsValue> {
    let Some(checkbox) = window
        .document()
        .ok_or("no document")?
        .get_element_by_id(element_id)
    else {
        return Ok(());
    };
    let checkbox = checkbox.dyn_into::<HtmlInputElement>()?;
    let state = state.clone();
    let checkbox_el = checkbox.clone();
    let handler = Closure::<dyn FnMut()>::new(move || {
        let mut state = state.borrow_mut();
        apply(&mut state, checkbox_el.checked());
        render(&state);
    });
    checkbox.add_event_listener_with_callback("change", handler.as_ref().unchecked_ref())?;
    handler.forget();
    Ok(())
}

fn on_pointer_down(state: &Rc<RefCell<AppState>>, event: PointerEvent) {
    event.prevent_default();
    let mut state = state.borrow_mut();
    let pos = state.canvas_pos(&event);
    let pointer_id = event.pointer_id();
    let _ = state.canvas.set_pointer_capture(pointer_id);

    if event.pointer_type() == "touch" {
        state.touch_points.insert(pointer_id, pos);
        match state.touch_points.len() {
            1 => {
                state.drawing_pointer_id = Some(pointer_id);
                begin_primary_press(&mut state, pos);
            }
            2 => {
                // A second finger turns this into a pan/zoom gesture; any
                // single-finger stroke or edit in progress is abandoned.
                state.current_stroke = None;
                state.click_candidate = None;
                state.press_pos = None;
                state.dragging_handle = None;
                state.drawing_pointer_id = None;
                state.gesture_prev = Some(gesture_state(&state.touch_points));
            }
            _ => {}
        }
    } else {
        match event.button() {
            0 => {
                state.drawing_pointer_id = Some(pointer_id);
                begin_primary_press(&mut state, pos);
            }
            1 => {
                state.pan_pointer_id = Some(pointer_id);
                state.last_pan_pos = pos;
            }
            _ => {}
        }
    }
}

fn on_pointer_move(state: &Rc<RefCell<AppState>>, event: PointerEvent) {
    let mut state = state.borrow_mut();
    let pos = state.canvas_pos(&event);
    let pointer_id = event.pointer_id();
    let mut dirty = false;

    if let std::collections::hash_map::Entry::Occupied(mut e) = state.touch_points.entry(pointer_id)
    {
        e.insert(pos);
        if state.touch_points.len() == 2 {
            let (mid, dist) = gesture_state(&state.touch_points);
            if let Some((prev_mid, prev_dist)) = state.gesture_prev {
                state.view.pan_by(mid.0 - prev_mid.0, mid.1 - prev_mid.1);
                if prev_dist > 0.0 {
                    state.view.zoom_at(dist / prev_dist, mid.0, mid.1);
                }
            }
            state.gesture_prev = Some((mid, dist));
            dirty = true;
        } else if state.drawing_pointer_id == Some(pointer_id) {
            dirty = continue_primary_press(&mut state, pos);
        }
    } else if state.pan_pointer_id == Some(pointer_id) {
        let (last_x, last_y) = state.last_pan_pos;
        state.view.pan_by(pos.0 - last_x, pos.1 - last_y);
        state.last_pan_pos = pos;
        dirty = true;
    } else if state.drawing_pointer_id == Some(pointer_id) {
        dirty = continue_primary_press(&mut state, pos);
    }

    if dirty {
        render(&state);
    }
}

fn on_pointer_up(state: &Rc<RefCell<AppState>>, event: PointerEvent) {
    let mut state = state.borrow_mut();
    let pos = state.canvas_pos(&event);
    let pointer_id = event.pointer_id();
    let _ = state.canvas.release_pointer_capture(pointer_id);

    if state.touch_points.remove(&pointer_id).is_some() && state.touch_points.len() < 2 {
        state.gesture_prev = None;
    }
    if state.drawing_pointer_id == Some(pointer_id) {
        state.drawing_pointer_id = None;
        state.dragging_handle = None;
        if let Some(idx) = state.click_candidate.take() {
            let moved = state.press_pos.take().map_or(0.0, |(px, py)| {
                ((pos.0 - px).powi(2) + (pos.1 - py).powi(2)).sqrt()
            });
            if moved <= CLICK_MOVE_THRESHOLD_PX {
                if let Some(stroke) = state.strokes.get_mut(idx) {
                    convert_to_bezier(stroke);
                }
            }
        } else if let Some(stroke) = state.current_stroke.take() {
            if stroke.len() > 1 {
                let layer = state.active_layer;
                state.strokes.push(stroke);
                state.stroke_layers.push(layer);
            }
        }
    }
    if state.pan_pointer_id == Some(pointer_id) {
        state.pan_pointer_id = None;
    }

    render(&state);
}

fn on_wheel(state: &Rc<RefCell<AppState>>, event: WheelEvent) {
    event.prevent_default();
    let mut state = state.borrow_mut();
    let rect = state.canvas.get_bounding_client_rect();
    let x = (event.client_x() as f64 - rect.left()) * state.dpr;
    let y = (event.client_y() as f64 - rect.top()) * state.dpr;
    // Exponential response so repeated wheel ticks feel evenly spaced.
    let factor = (-event.delta_y() * 0.001).exp();
    state.view.zoom_at(factor, x, y);
    render(&state);
}

/// World units per TikZ coordinate unit. Keeps generated coordinates at a
/// reasonable magnitude regardless of how zoomed-in the drawing was.
const TIKZ_SCALE: f64 = 100.0;
/// TikZ line widths, in points, for the ink and outline passes — the
/// outline wider than the ink so it reads as a halo behind it, the same
/// relationship `LINE_WIDTH`/`OUTLINE_WIDTH` give the canvas rendering.
const TIKZ_INK_WIDTH_PT: f64 = 0.8;
const TIKZ_OUTLINE_WIDTH_PT: f64 = 2.4;

/// Build just the coordinate portion of a stroke's TikZ path (no leading
/// `\draw` or trailing `;`) — a straight `--` for a line, a `.. controls
/// .. and ..` Bezier for a curve — so it can be reused for both the
/// outline and ink passes. The y axis is flipped because canvas
/// coordinates grow downward while TikZ's grow upward. `None` for an
/// in-progress stroke, which never reaches `AppState::strokes`.
fn stroke_tikz_path(stroke: &Stroke) -> Option<String> {
    match stroke[..] {
        [(x0, y0), (x1, y1)] => Some(format!(
            "({:.2},{:.2}) -- ({:.2},{:.2})",
            x0 / TIKZ_SCALE,
            -y0 / TIKZ_SCALE,
            x1 / TIKZ_SCALE,
            -y1 / TIKZ_SCALE,
        )),
        // p0, p1, p2, p3 are already in order along the curve, so they
        // carry straight over into TikZ's own point order.
        [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] => Some(format!(
            "({:.2},{:.2}) .. controls ({:.2},{:.2}) and ({:.2},{:.2}) .. ({:.2},{:.2})",
            x0 / TIKZ_SCALE,
            -y0 / TIKZ_SCALE,
            x1 / TIKZ_SCALE,
            -y1 / TIKZ_SCALE,
            x2 / TIKZ_SCALE,
            -y2 / TIKZ_SCALE,
            x3 / TIKZ_SCALE,
            -y3 / TIKZ_SCALE,
        )),
        _ => None,
    }
}

/// Render the drawing as a TikZ picture, matching the canvas's layout but
/// always in black and white regardless of the on-screen ink/background
/// colors: each stroke gets an arrowhead and a wider white outline behind
/// its black ink. Strokes are emitted lowest layer first and, within a
/// layer, every outline before any ink in that layer — the same scheme
/// `render` uses on canvas — so a higher layer's outline still visually
/// cuts a gap in a lower layer's line, but same-layer strokes don't cut
/// into each other.
fn strokes_to_tikz(state: &AppState) -> String {
    let mut tikz = String::from("\\begin{tikzpicture}\n");
    for layer in 0..NUM_LAYERS {
        let indices: Vec<usize> = (0..state.strokes.len())
            .filter(|&i| state.stroke_layers[i] == layer)
            .collect();
        for &i in &indices {
            if let Some(path) = stroke_tikz_path(&state.strokes[i]) {
                tikz.push_str(&format!(
                    "  \\draw[->, draw=white, line width={TIKZ_OUTLINE_WIDTH_PT}pt] {path};\n"
                ));
            }
        }
        for &i in &indices {
            if let Some(path) = stroke_tikz_path(&state.strokes[i]) {
                tikz.push_str(&format!(
                    "  \\draw[->, draw=black, line width={TIKZ_INK_WIDTH_PT}pt] {path};\n"
                ));
            }
        }
    }
    tikz.push_str("\\end{tikzpicture}\n");
    tikz
}

/// Fill `#tikz-output` with the drawing's current TikZ and reveal
/// `#tikz-panel`, so the user can read it or copy it to the clipboard.
fn show_tikz_panel(state: &AppState) -> Result<(), JsValue> {
    let tikz = strokes_to_tikz(state);
    let document = web_sys::window()
        .ok_or("no global window")?
        .document()
        .ok_or("no document")?;
    if let Some(textarea) = document.get_element_by_id("tikz-output") {
        textarea.dyn_into::<HtmlTextAreaElement>()?.set_value(&tikz);
    }
    if let Some(panel) = document.get_element_by_id("tikz-panel") {
        panel.remove_attribute("hidden")?;
    }
    Ok(())
}

/// Hide `#tikz-panel`.
fn hide_tikz_panel() -> Result<(), JsValue> {
    let document = web_sys::window()
        .ok_or("no global window")?
        .document()
        .ok_or("no document")?;
    if let Some(panel) = document.get_element_by_id("tikz-panel") {
        panel.set_attribute("hidden", "")?;
    }
    Ok(())
}

/// Copy `#tikz-output`'s current contents to the clipboard.
fn copy_tikz_to_clipboard() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no global window")?;
    let document = window.document().ok_or("no document")?;
    let Some(textarea) = document.get_element_by_id("tikz-output") else {
        return Ok(());
    };
    let textarea = textarea.dyn_into::<HtmlTextAreaElement>()?;
    // Fire-and-forget: the returned promise can be dropped, since the
    // clipboard write still completes asynchronously without being awaited.
    let _ = window.navigator().clipboard().write_text(&textarea.value());
    Ok(())
}
