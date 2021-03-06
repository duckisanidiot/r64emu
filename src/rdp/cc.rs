// Color combiner

// TODO:
//   * 2-cycle mode
//   * chroma key
//   * coverage alpha
//   * alpha dithering

extern crate bit_field;
extern crate emu;

use self::bit_field::BitField;
use super::{MColor, MultiColor};
use emu::gfx::{Color, Rgba8888};
use std::fmt;
use std::ptr;

struct CombinerCycle {
    suba: *const MultiColor,
    subb: *const MultiColor,
    mul: *const MultiColor,
    add: *const MultiColor,
}

impl Default for CombinerCycle {
    fn default() -> CombinerCycle {
        CombinerCycle {
            suba: ptr::null(),
            subb: ptr::null(),
            mul: ptr::null(),
            add: ptr::null(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Combiner {
    combined: MultiColor,
    texel0: MultiColor,
    texel1: MultiColor,
    prim: MultiColor,
    shade: MultiColor,
    env: MultiColor,
    key_center: MultiColor,
    key_scale: MultiColor,
    lod_fraction: MultiColor,
    prim_lod_fraction: MultiColor,
    noise: MultiColor,
    conv_k4: MultiColor,
    conv_k5: MultiColor,
    one: MultiColor,
    zero: MultiColor,

    cycle_rgb: [CombinerCycle; 2],
    cycle_alpha: [CombinerCycle; 2],
}

struct CombinerMode(u64);
impl CombinerMode {
    #[inline]
    fn cyc0_rgb(&self) -> (u32, u32, u32, u32) {
        (
            self.0.get_bits(52..56) as u32,
            self.0.get_bits(28..32) as u32,
            self.0.get_bits(47..52) as u32,
            self.0.get_bits(15..18) as u32,
        )
    }

    #[inline]
    fn cyc0_alpha(&self) -> (u32, u32, u32, u32) {
        (
            self.0.get_bits(44..47) as u32,
            self.0.get_bits(12..15) as u32,
            self.0.get_bits(41..44) as u32,
            self.0.get_bits(9..12) as u32,
        )
    }

    #[inline]
    fn cyc1_rgb(&self) -> (u32, u32, u32, u32) {
        (
            self.0.get_bits(37..41) as u32,
            self.0.get_bits(24..28) as u32,
            self.0.get_bits(32..37) as u32,
            self.0.get_bits(6..9) as u32,
        )
    }

    #[inline]
    fn cyc1_alpha(&self) -> (u32, u32, u32, u32) {
        (
            self.0.get_bits(21..24) as u32,
            self.0.get_bits(3..6) as u32,
            self.0.get_bits(18..21) as u32,
            self.0.get_bits(0..3) as u32,
        )
    }
}

impl Combiner {
    pub(crate) fn new() -> Combiner {
        Combiner {
            one: MultiColor::splat(0x100),
            lod_fraction: MultiColor::splat(0xFF),
            ..Default::default()
        }
    }

    #[inline(always)]
    fn combine_cycle(&mut self, cyc: usize) -> MultiColor {
        let (suba, subb, mul, add) = unsafe {
            (
                *self.cycle_rgb[cyc].suba,
                *self.cycle_rgb[cyc].subb,
                *self.cycle_rgb[cyc].mul,
                *self.cycle_rgb[cyc].add,
            )
        };
        let rgb: MultiColor = (suba - subb) * mul + (add << 8) + MultiColor::splat(0x80);

        let (suba, subb, mul, add) = unsafe {
            (
                *self.cycle_alpha[cyc].suba,
                *self.cycle_alpha[cyc].subb,
                *self.cycle_alpha[cyc].mul,
                *self.cycle_alpha[cyc].add,
            )
        };
        let alpha: MultiColor = (suba - subb) * mul + (add << 8) + MultiColor::splat(0x80);
        rgb.replace_alpha(alpha) >> 8
    }

    #[inline(always)]
    pub(crate) fn combine_1cycle(&mut self, shade: MultiColor) -> MultiColor {
        self.shade = shade;
        let c = self.combine_cycle(1);

        // Save as combined color (FIXME: this is not correct with parallel pixels)
        self.combined = c;

        return c;
    }

    unsafe fn setup_cycle_basic(&self, v: u32) -> *const MultiColor {
        match v {
            0 => &self.combined,
            1 => &self.texel0,
            2 => &self.texel1,
            3 => &self.prim,
            4 => &self.shade,
            5 => &self.env,
            _ => unreachable!(),
        }
    }

    unsafe fn setup_cycle_rgb(
        &self,
        (suba, subb, mul, add): (u32, u32, u32, u32),
    ) -> CombinerCycle {
        CombinerCycle {
            suba: match suba {
                0...5 => self.setup_cycle_basic(suba),
                6 => &self.one,
                7 => &self.noise,
                8...15 => &self.zero,
                _ => unreachable!(),
            },
            subb: match subb {
                0...5 => self.setup_cycle_basic(subb),
                6 => &self.key_center,
                7 => &self.conv_k4,
                8...15 => &self.zero,
                _ => unreachable!(),
            },
            mul: match mul {
                0...5 => self.setup_cycle_basic(mul),
                6 => &self.key_scale,
                7 => &self.combined,
                8 => &self.texel0,
                9 => &self.texel1,
                10 => &self.prim,
                11 => &self.shade,
                12 => &self.env,
                13 => &self.lod_fraction,
                14 => &self.prim_lod_fraction,
                15 => &self.conv_k5,
                16...31 => &self.zero,
                _ => unreachable!(),
            },
            add: match add {
                0...5 => self.setup_cycle_basic(add),
                6 => &self.one,
                _ => &self.zero,
            },
        }
    }

    unsafe fn setup_cycle_alpha(
        &self,
        (suba, subb, mul, add): (u32, u32, u32, u32),
    ) -> CombinerCycle {
        CombinerCycle {
            suba: match suba {
                0...5 => self.setup_cycle_basic(suba),
                6 => &self.one,
                7...15 => &self.zero,
                _ => unreachable!(),
            },
            subb: match subb {
                0...5 => self.setup_cycle_basic(subb),
                6 => &self.one,
                7...15 => &self.zero,
                _ => unreachable!(),
            },
            mul: match mul {
                0 => &self.lod_fraction,
                1...5 => self.setup_cycle_basic(mul),
                6 => &self.prim_lod_fraction,
                7...31 => &self.zero,
                _ => unreachable!(),
            },
            add: match add {
                0...5 => self.setup_cycle_basic(add),
                6 => &self.one,
                7...15 => &self.zero,
                _ => unreachable!(),
            },
        }
    }
    pub(crate) fn set_mode(&mut self, mode: u64) {
        let mode = CombinerMode(mode);

        self.cycle_rgb[0] = unsafe { self.setup_cycle_rgb(mode.cyc0_rgb()) };
        self.cycle_rgb[1] = unsafe { self.setup_cycle_rgb(mode.cyc1_rgb()) };

        self.cycle_alpha[0] = unsafe { self.setup_cycle_alpha(mode.cyc0_alpha()) };
        self.cycle_alpha[1] = unsafe { self.setup_cycle_alpha(mode.cyc1_alpha()) };
    }

    pub(crate) fn set_tex0(&mut self, c: MultiColor) {
        self.texel0 = c;
    }
    pub(crate) fn set_tex1(&mut self, c: MultiColor) {
        self.texel1 = c;
    }
    pub(crate) fn set_prim(&mut self, c: Color<Rgba8888>) {
        self.prim = MultiColor::from_color(c);
    }
    pub(crate) fn set_shade(&mut self, c: Color<Rgba8888>) {
        self.shade = MultiColor::from_color(c);
    }
    pub(crate) fn set_env(&mut self, c: Color<Rgba8888>) {
        self.env = MultiColor::from_color(c);
    }

    fn repr_comb_ptr(&self, ptr: *const MultiColor) -> String {
        if ptr == &self.combined {
            "combined".into()
        } else if ptr == &self.texel0 {
            "tex0".into()
        } else if ptr == &self.texel1 {
            "tex1".into()
        } else if ptr == &self.prim {
            "prim".into()
        } else if ptr == &self.shade {
            "shade".into()
        } else if ptr == &self.env {
            "env".into()
        } else if ptr == &self.key_center {
            "key_center".into()
        } else if ptr == &self.key_scale {
            "key_scale".into()
        } else if ptr == &self.lod_fraction {
            "lod_fraction".into()
        } else if ptr == &self.prim_lod_fraction {
            "prim_lod_fraction".into()
        } else if ptr == &self.noise {
            "noise".into()
        } else if ptr == &self.conv_k4 {
            "K4".into()
        } else if ptr == &self.conv_k5 {
            "K5".into()
        } else if ptr == &self.one {
            "1.0".into()
        } else if ptr == &self.zero {
            "0.0".into()
        } else {
            "?".into()
        }
    }

    pub(crate) fn fmt_1cycle(&self) -> String {
        format!(
            "Combiner {{ rgb: ({}-{})*{}+{}, alpha: ({}-{})*{}+{} }}",
            self.repr_comb_ptr(self.cycle_rgb[1].suba),
            self.repr_comb_ptr(self.cycle_rgb[1].subb),
            self.repr_comb_ptr(self.cycle_rgb[1].mul),
            self.repr_comb_ptr(self.cycle_rgb[1].add),
            self.repr_comb_ptr(self.cycle_alpha[1].suba),
            self.repr_comb_ptr(self.cycle_alpha[1].subb),
            self.repr_comb_ptr(self.cycle_alpha[1].mul),
            self.repr_comb_ptr(self.cycle_alpha[1].add),
        )
    }
}
