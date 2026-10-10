//! Non-power-of-two flash and >128 KB (22-bit PC) devices, per the AVR Instruction Set Manual
//! (DS40002198B). Specs are derived from the ATmega328P by enlarging the flash.

use mcs_core::avr::device::{AvrDeviceSpec, IoRegisterSpec, RegisterAccess};
use mcs_core::avr::devices;
use mcs_core::avr::isa::{def_by_op, encode, feature, op};
use mcs_core::program::{LoadedProgram, ProgramFormat};
use mcs_sim::avr::Machine;

fn spec(flash: u32, ext_regs: bool) -> &'static AvrDeviceSpec {
    let mut s = devices::get("atmega328p").unwrap().clone();
    s.flash_size = flash;
    if ext_regs {
        s.features |= feature::JMP | feature::EIJMP | feature::ELPM | feature::ELPMX;
        for (name, addr) in [("RAMPZ", 0x5b), ("EIND", 0x5c)] {
            s.registers.push(IoRegisterSpec {
                name: name.into(),
                addr,
                reset: 0,
                group: "CPU".into(),
                desc: String::new(),
                bits: vec![],
                access: RegisterAccess::Rw,
            });
        }
    }
    Box::leak(Box::new(s))
}

fn put(img: &mut [u8], word: usize, insns: &[(u8, &[i32])]) {
    let mut w = word;
    for (o, vals) in insns {
        for x in encode(def_by_op(*o).unwrap(), vals).unwrap() {
            img[w * 2] = x as u8;
            img[w * 2 + 1] = (x >> 8) as u8;
            w += 1;
        }
    }
}

fn machine(s: &'static AvrDeviceSpec, img: Vec<u8>) -> Machine {
    let mut m = Machine::new(s);
    let mut p = LoadedProgram::empty(ProgramFormat::Asm, s.flash_size as usize);
    p.flash = img;
    m.load(&p);
    m
}

#[test]
fn call_ret_22bit_pc() {
    let s = spec(256 * 1024, true);
    let mut img = vec![0xff; s.flash_size as usize];
    put(&mut img, 0, &[(op::CALL, &[0x18000]), (op::NOP, &[])]);
    put(&mut img, 0x18000, &[(op::RET, &[])]);
    let mut m = machine(s, img);
    let sp0 = m.cpu.sp;
    let c0 = m.cpu.cycles;
    m.step();
    assert_eq!(m.cpu.pc, 0x18000);
    assert_eq!(m.cpu.sp, sp0 - 3);
    assert_eq!(m.cpu.cycles - c0, 5);
    // Big-endian in memory: low byte at the highest address (pushed first).
    assert_eq!(m.cpu.data[sp0 as usize], 2);
    m.step();
    assert_eq!(m.cpu.pc, 2);
    assert_eq!(m.cpu.sp, sp0);
    assert_eq!(m.cpu.cycles - c0, 10);
}

#[test]
fn rcall_cycles_22bit() {
    let s = spec(256 * 1024, true);
    let mut img = vec![0xff; s.flash_size as usize];
    put(&mut img, 0, &[(op::RCALL, &[1])]);
    let mut m = machine(s, img);
    m.step();
    assert_eq!((m.cpu.pc, m.cpu.cycles), (2, 4));
}

#[test]
fn eicall_and_elpm() {
    let s = spec(256 * 1024, true);
    let mut img = vec![0xff; s.flash_size as usize];
    put(
        &mut img,
        0,
        &[
            (op::LDI, &[16, 1]),
            (op::STS, &[0x5c, 16]), // EIND = 1
            (op::LDI, &[30, 0x34]),
            (op::LDI, &[31, 0x12]),
            (op::EICALL, &[]),
        ],
    );
    put(&mut img, 0x11234, &[(op::NOP, &[])]);
    let mut m = machine(s, img);
    let sp0 = m.cpu.sp;
    for _ in 0..4 {
        m.step();
    }
    let c = m.cpu.cycles;
    m.step();
    assert_eq!(m.cpu.pc, 0x11234);
    assert_eq!(m.cpu.sp, sp0 - 3);
    assert_eq!(m.cpu.cycles - c, 4);

    // ELPM with RAMPZ=1 reads above 64 KB; ELPM Z+ carries into RAMPZ.
    let mut img = vec![0xff; s.flash_size as usize];
    put(&mut img, 0, &[(op::ELPM_ZP, &[5, 0]), (op::ELPM_ZP, &[6, 0])]);
    img[0x1ffff] = 0xab;
    img[0x20000] = 0xcd;
    let mut m = machine(s, img);
    m.cpu.data[0x5b] = 1;
    m.cpu.r[30] = 0xff;
    m.cpu.r[31] = 0xff;
    m.step();
    assert_eq!(m.cpu.r[5], 0xab);
    assert_eq!((m.cpu.r[30], m.cpu.r[31], m.cpu.data[0x5b]), (0, 0, 2));
    m.step();
    assert_eq!(m.cpu.r[6], 0xcd);
}

#[test]
fn non_power_of_two_flash() {
    let s = spec(3000, false);
    let mut img = vec![0xff; 3000];
    put(&mut img, 0, &[(op::RJMP, &[-1])]);
    let mut m = machine(s, img);
    assert_eq!(m.cpu.flash.len(), 3000);
    assert_eq!(m.cpu.pc_mask, 2047);
    m.run(1000);
    // Execute into the erased padding area and wrap without panicking.
    m.cpu.pc = 1499;
    m.run(10_000);
    m.cpu.write_flash_byte(5000, 1);
    m.cpu.load_flash(&vec![0u8; 4000]);
}
