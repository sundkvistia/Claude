mod view;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use view::View;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, PointerEvent, WheelEvent};

type Point = (f64, f64);
type Stroke = Vec<Point>;

/// Screen-space line width, in device pixels at zoom = 1.0. Because the
/// canvas context is scaled by the current zoom before strokes are drawn,
/// the ink itself scales with the drawing (like real ink on paper) rather
/// than staying a fixed width on screen.
const LINE_WIDTH: f64 = 3.0;
const BACKGROUND: &str = "#1e1e1e";
const INK: &str = "#f0f0f0";

struct AppState {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    dpr: f64,
    view: View,

    strokes: Vec<Stroke>,
    current_stroke: Option<Stroke>,
    /// Pointer id currently drawing (left mouse button, pen, or a lone touch).
    drawing_pointer_id: Option<i32>,
    /// Pointer id currently panning (middle mouse button).
    pan_pointer_id: Option<i32>,
    last_pan_pos: Point,

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
        strokes: Vec::new(),
        current_stroke: None,
        drawing_pointer_id: None,
        pan_pointer_id: None,
        last_pan_pos: (0.0, 0.0),
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
    state
        .canvas
        .set_width((width * state.dpr).max(1.0) as u32);
    state
        .canvas
        .set_height((height * state.dpr).max(1.0) as u32);
}

fn render(state: &AppState) {
    let ctx = &state.ctx;
    let width = state.canvas.width() as f64;
    let height = state.canvas.height() as f64;

    ctx.set_fill_style_str(BACKGROUND);
    ctx.fill_rect(0.0, 0.0, width, height);

    ctx.save();
    ctx.translate(state.view.pan_x, state.view.pan_y)
        .expect("translate");
    ctx.scale(state.view.zoom, state.view.zoom).expect("scale");

    ctx.set_stroke_style_str(INK);
    ctx.set_line_width(LINE_WIDTH);
    ctx.set_line_cap("round");
    ctx.set_line_join("round");

    for stroke in state.strokes.iter().chain(state.current_stroke.iter()) {
        draw_stroke(ctx, stroke);
    }

    ctx.restore();
}

fn draw_stroke(ctx: &CanvasRenderingContext2d, stroke: &Stroke) {
    if stroke.len() < 2 {
        return;
    }
    ctx.begin_path();
    let (x0, y0) = stroke[0];
    ctx.move_to(x0, y0);
    for &(x, y) in &stroke[1..] {
        ctx.line_to(x, y);
    }
    ctx.stroke();
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
    {
        let state = state.clone();
        let handler = Closure::<dyn FnMut()>::new(move || {
            let mut state = state.borrow_mut();
            resize_canvas(&state);
            render(&mut state);
        });
        window.add_event_listener_with_callback("resize", handler.as_ref().unchecked_ref())?;
        handler.forget();
    }
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
                state.current_stroke = Some(vec![state.view.screen_to_world(pos.0, pos.1)]);
            }
            2 => {
                // A second finger turns this into a pan/zoom gesture; any
                // single-finger stroke in progress is abandoned.
                state.current_stroke = None;
                state.drawing_pointer_id = None;
                state.gesture_prev = Some(gesture_state(&state.touch_points));
            }
            _ => {}
        }
    } else {
        match event.button() {
            0 => {
                state.drawing_pointer_id = Some(pointer_id);
                state.current_stroke = Some(vec![state.view.screen_to_world(pos.0, pos.1)]);
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

    if state.touch_points.contains_key(&pointer_id) {
        state.touch_points.insert(pointer_id, pos);
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
            let world = state.view.screen_to_world(pos.0, pos.1);
            if let Some(stroke) = state.current_stroke.as_mut() {
                stroke.push(world);
                dirty = true;
            }
        }
    } else if state.pan_pointer_id == Some(pointer_id) {
        let (last_x, last_y) = state.last_pan_pos;
        state.view.pan_by(pos.0 - last_x, pos.1 - last_y);
        state.last_pan_pos = pos;
        dirty = true;
    } else if state.drawing_pointer_id == Some(pointer_id) {
        let world = state.view.screen_to_world(pos.0, pos.1);
        if let Some(stroke) = state.current_stroke.as_mut() {
            stroke.push(world);
            dirty = true;
        }
    }

    if dirty {
        render(&state);
    }
}

fn on_pointer_up(state: &Rc<RefCell<AppState>>, event: PointerEvent) {
    let mut state = state.borrow_mut();
    let pointer_id = event.pointer_id();
    let _ = state.canvas.release_pointer_capture(pointer_id);

    if state.touch_points.remove(&pointer_id).is_some() {
        if state.touch_points.len() < 2 {
            state.gesture_prev = None;
        }
    }
    if state.drawing_pointer_id == Some(pointer_id) {
        state.drawing_pointer_id = None;
        if let Some(stroke) = state.current_stroke.take() {
            if stroke.len() > 1 {
                state.strokes.push(stroke);
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
