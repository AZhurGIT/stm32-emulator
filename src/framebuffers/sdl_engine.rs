// SPDX-License-Identifier: GPL-3.0-or-later

use std::{sync::Mutex, rc::Rc, cell::RefCell};

use sdl2::{
    event::Event,
    keyboard::Keycode,
    keyboard::Scancode,
    EventPump, VideoSubsystem, render::Canvas, video::Window, pixels,
};

lazy_static::lazy_static! {
    pub static ref SDL: Mutex<SdlEngine> = Mutex::new(SdlEngine::new());
    static ref VIRTUAL_INPUTS: Mutex<VirtualInputs> = Mutex::new(VirtualInputs::default());
}

#[derive(Clone, Copy)]
pub enum VirtualInputPin {
    LeftEncoderA,
    LeftEncoderB,
    RightEncoderA,
    RightEncoderB,
    LeftButton,
    RightButton,
}

#[derive(Default)]
struct VirtualInputs {
    left_encoder_state: u8,
    right_encoder_state: u8,
    left_button_pressed: bool,
    right_button_pressed: bool,
}

fn encoder_phase_value(phase: u8) -> (bool, bool) {
    // Gray sequence for quadrature encoder.
    match phase & 0b11 {
        0b00 => (false, false),
        0b01 => (false, true),
        0b10 => (true, true),
        0b11 => (true, false),
        _ => unreachable!(),
    }
}

impl VirtualInputs {
    fn encoder_step_left(&mut self, clockwise: bool) {
        self.left_encoder_state = if clockwise {
            self.left_encoder_state.wrapping_add(1)
        } else {
            self.left_encoder_state.wrapping_sub(1)
        };
    }

    fn encoder_step_right(&mut self, clockwise: bool) {
        self.right_encoder_state = if clockwise {
            self.right_encoder_state.wrapping_add(1)
        } else {
            self.right_encoder_state.wrapping_sub(1)
        };
    }

    fn pin_level(&self, pin: VirtualInputPin) -> bool {
        let (la, lb) = encoder_phase_value(self.left_encoder_state);
        let (ra, rb) = encoder_phase_value(self.right_encoder_state);
        match pin {
            VirtualInputPin::LeftEncoderA => la,
            VirtualInputPin::LeftEncoderB => lb,
            VirtualInputPin::RightEncoderA => ra,
            VirtualInputPin::RightEncoderB => rb,
            // Buttons are pull-up and active low.
            VirtualInputPin::LeftButton => !self.left_button_pressed,
            VirtualInputPin::RightButton => !self.right_button_pressed,
        }
    }
}

pub fn virtual_input_level(pin: VirtualInputPin) -> bool {
    VIRTUAL_INPUTS.lock().unwrap().pin_level(pin)
}

pub struct SdlEngine {
    event_pump: EventPump,
    video_subsystem: VideoSubsystem,
}

/// How often should we call pump_events() in terms of number of instructions emulated
pub const PUMP_EVENT_INST_INTERVAL: u64 = 100_000; // ~1-10ms, depending on the speed of the host

unsafe impl Send for SdlEngine {}
unsafe impl Sync for SdlEngine {}

impl SdlEngine {
    pub fn new() -> Self {
        let sdl_context = sdl2::init().unwrap();
        let video_subsystem = sdl_context.video().unwrap();

        let event_pump = sdl_context.event_pump().unwrap();

        Self { event_pump, video_subsystem }
    }

    pub fn new_canvas(&mut self, title: &str, width: u32, height: u32) -> Canvas<Window> {
        let window = self.video_subsystem.window(title, width, height)
            .resizable()
            .build()
            .unwrap();

        let mut canvas = window.into_canvas().build().unwrap();

        canvas.set_draw_color(pixels::Color::RGB(0, 0, 0));
        canvas.clear();
        canvas.present();

        canvas
    }

    /// Returns false if we need to quit
    pub fn pump_events(&mut self, framebuffers: &[Rc<RefCell<super::Sdl>>]) -> bool {
        for event in self.event_pump.poll_iter() {
            let is_sc = |sc: Option<Scancode>, want: Scancode| sc == Some(want);
            match event {
                Event::Quit {..} |
                Event::KeyDown { keycode: Some(Keycode::Q), .. } |
                Event::KeyDown { keycode: Some(Keycode::Escape), .. } => {
                    return false;
                },
                Event::KeyDown { keycode, scancode, repeat: false, .. }
                    if matches!(keycode, Some(Keycode::A | Keycode::Left))
                        || is_sc(scancode, Scancode::A) =>
                {
                    VIRTUAL_INPUTS.lock().unwrap().encoder_step_left(false);
                }
                Event::KeyDown { keycode, scancode, repeat: false, .. }
                    if matches!(keycode, Some(Keycode::D | Keycode::Right))
                        || is_sc(scancode, Scancode::D) =>
                {
                    VIRTUAL_INPUTS.lock().unwrap().encoder_step_left(true);
                }
                Event::KeyDown { keycode, scancode, repeat: false, .. }
                    if matches!(keycode, Some(Keycode::J | Keycode::Down))
                        || is_sc(scancode, Scancode::J) =>
                {
                    VIRTUAL_INPUTS.lock().unwrap().encoder_step_right(false);
                }
                Event::KeyDown { keycode, scancode, repeat: false, .. }
                    if matches!(keycode, Some(Keycode::L | Keycode::Up))
                        || is_sc(scancode, Scancode::L) =>
                {
                    VIRTUAL_INPUTS.lock().unwrap().encoder_step_right(true);
                }
                Event::KeyDown { keycode, scancode, .. }
                    if matches!(keycode, Some(Keycode::Num1 | Keycode::Kp1))
                        || is_sc(scancode, Scancode::Num1)
                        || is_sc(scancode, Scancode::Kp1) =>
                {
                    VIRTUAL_INPUTS.lock().unwrap().left_button_pressed = true;
                }
                Event::KeyUp { keycode, scancode, .. }
                    if matches!(keycode, Some(Keycode::Num1 | Keycode::Kp1))
                        || is_sc(scancode, Scancode::Num1)
                        || is_sc(scancode, Scancode::Kp1) =>
                {
                    VIRTUAL_INPUTS.lock().unwrap().left_button_pressed = false;
                }
                Event::KeyDown { keycode: Some(Keycode::Z), .. } => {
                    VIRTUAL_INPUTS.lock().unwrap().left_button_pressed = true;
                }
                Event::KeyUp { keycode: Some(Keycode::Z), .. } => {
                    VIRTUAL_INPUTS.lock().unwrap().left_button_pressed = false;
                }
                Event::KeyDown { keycode, scancode, .. }
                    if matches!(keycode, Some(Keycode::Num2 | Keycode::Kp2))
                        || is_sc(scancode, Scancode::Num2)
                        || is_sc(scancode, Scancode::Kp2) =>
                {
                    VIRTUAL_INPUTS.lock().unwrap().right_button_pressed = true;
                }
                Event::KeyUp { keycode, scancode, .. }
                    if matches!(keycode, Some(Keycode::Num2 | Keycode::Kp2))
                        || is_sc(scancode, Scancode::Num2)
                        || is_sc(scancode, Scancode::Kp2) =>
                {
                    VIRTUAL_INPUTS.lock().unwrap().right_button_pressed = false;
                }
                Event::KeyDown { keycode: Some(Keycode::X), .. } => {
                    VIRTUAL_INPUTS.lock().unwrap().right_button_pressed = true;
                }
                Event::KeyUp { keycode: Some(Keycode::X), .. } => {
                    VIRTUAL_INPUTS.lock().unwrap().right_button_pressed = false;
                }
                Event::MouseMotion { ref window_id, .. } |
                Event::MouseButtonDown { ref window_id, .. } |
                Event::MouseButtonUp { ref window_id, .. } => {
                    if let Some(fb) = framebuffers.iter().find(|fb| fb.borrow().window_id == *window_id) {
                        fb.borrow_mut().process_event(event);
                    }
                }
                _ => {}
            }
        }
        true
    }
}
