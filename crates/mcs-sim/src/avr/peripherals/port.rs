//! GPIO port (PINx / DDRx / PORTx / PUEx).

use crate::avr::machine::{Cx, Peripheral};

pub struct PortConfig {
    pub name: &'static str,
    pub pin: u16,
    pub ddr: u16,
    pub port: u16,
    /// Separate pull-up enable register (AVRrc). When absent, PORTx enables pull-ups on inputs.
    pub pue: Option<u16>,
    /// Digital input disable bit per port bit (register, mask); disabled inputs read as 0.
    pub didr: Vec<Option<(u16, u8)>>,
    /// Global pull-up disable bit (MCUCR.PUD on classic AVRs).
    pub pud: Option<(u16, u8)>,
    /// GPIO indices for bit 0..n.
    pub gpios: Vec<usize>,
    /// GPIO doubling as RESET while the RSTDISBL fuse is unprogrammed.
    pub reset_gpio: Option<usize>,
}

pub struct Port {
    c: PortConfig,
    mask: u8,
}

impl Port {
    pub fn new(c: PortConfig) -> Self {
        let mask = ((1u16 << c.gpios.len()) - 1) as u8;
        Self { c, mask }
    }

    pub fn registers(&self) -> Vec<u16> {
        let mut v = vec![self.c.pin, self.c.ddr, self.c.port];
        v.extend(self.c.pue);
        v
    }

    fn read_pin(&self, cx: &Cx) -> u8 {
        let mut v = 0u8;
        for (i, &g) in self.c.gpios.iter().enumerate() {
            let disabled = self.c.didr.get(i).copied().flatten().is_some_and(|(r, m)| cx.cpu.data[r as usize] & m != 0);
            if !disabled {
                v |= cx.sys.pins[g].level << i;
            }
        }
        v & self.mask
    }

    fn apply(&mut self, cx: &mut Cx) {
        let d = &cx.cpu.data;
        let ddr = d[self.c.ddr as usize];
        let port = d[self.c.port as usize];
        let pud = self.c.pud.is_some_and(|(r, m)| d[r as usize] & m != 0);
        let pue = match self.c.pue {
            Some(a) => d[a as usize],
            None if pud => 0,
            None => !ddr & port,
        };
        let now = cx.cpu.cycles;
        for (i, &g) in self.c.gpios.iter().enumerate() {
            let p = &mut cx.sys.pins[g];
            let dir = (ddr >> i) & 1;
            let out = (port >> i) & 1;
            let pu = if p.reserved { 1 } else { (pue >> i) & 1 & (dir ^ 1) };
            if p.dir != dir || p.out != out || p.pullup != pu {
                p.dir = dir;
                p.out = out;
                p.pullup = pu;
                cx.sys.update_pin(g, now);
            }
        }
    }
}

impl Peripheral for Port {
    fn name(&self) -> &str {
        self.c.name
    }

    fn read(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        if addr == self.c.pin { self.read_pin(cx) } else { cx.cpu.data[addr as usize] }
    }

    fn peek(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        self.read(addr, cx)
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        let v = v & self.mask;
        if addr == self.c.pin {
            // Writing 1 to PINx toggles the PORTx bit.
            if v != 0 {
                cx.cpu.data[self.c.port as usize] ^= v;
                self.apply(cx);
            }
            return;
        }
        cx.cpu.data[addr as usize] = v;
        self.apply(cx);
    }

    fn on_reg_written(&mut self, addr: u16, cx: &mut Cx) {
        // PUD changes, or PORTx bits toggled by another module (USI USITC).
        if addr == self.c.port || self.c.pud.is_some_and(|(r, _)| r == addr) {
            self.apply(cx);
        }
    }

    fn reset(&mut self, cx: &mut Cx) {
        if let Some(rg) = self.c.reset_gpio {
            let is_reset = !cx.fuse_programmed("RSTDISBL");
            let p = &mut cx.sys.pins[rg];
            p.reserved = is_reset;
            p.reserved_by = if is_reset { "RESET" } else { "" };
            cx.sys.reset_pin = is_reset.then_some(rg);
        }
        for &g in &self.c.gpios {
            let p = &mut cx.sys.pins[g];
            p.dir = 0;
            p.out = 0;
            p.pullup = p.reserved as u8;
        }
        self.apply(cx);
    }

    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        match cx.sys.reset_pin {
            Some(r) => vec![("RESET pin".into(), format!("{}{}", cx.sys.pins[r].name, if cx.sys.reset_held { " (held low)" } else { "" }))],
            None => vec![],
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
