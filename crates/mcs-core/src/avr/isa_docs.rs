//! Short reference text for every canonical AVR mnemonic (Instruction Set panel).
//! Source: Microchip "AVR Instruction Set Manual" (DS40002198B).

/// Reference entry: what the instruction does, its operation and the SREG flags it changes.
#[derive(Clone, Copy, Debug)]
pub struct InsnDoc {
    pub summary: &'static str,
    pub operation: &'static str,
    pub flags: &'static str,
    /// Assembler aliases that encode to this instruction.
    pub aliases: &'static str,
}

const fn d(summary: &'static str, operation: &'static str, flags: &'static str, aliases: &'static str) -> InsnDoc {
    InsnDoc { summary, operation, flags, aliases }
}

/// Reference entry for a canonical lower-case mnemonic.
pub fn insn_doc(name: &str) -> Option<InsnDoc> {
    Some(match name {
        "adc" => d("Add with Carry", "Rd <- Rd + Rr + C", "Z C N V S H", "ROL Rd (= ADC Rd,Rd)"),
        "add" => d("Add without Carry", "Rd <- Rd + Rr", "Z C N V S H", "LSL Rd (= ADD Rd,Rd)"),
        "adiw" => d("Add Immediate to Word", "Rd+1:Rd <- Rd+1:Rd + K", "Z C N V S", ""),
        "and" => d("Logical AND", "Rd <- Rd & Rr", "Z N V S", "TST Rd (= AND Rd,Rd)"),
        "andi" => d("Logical AND with Immediate", "Rd <- Rd & K", "Z N V S", "CBR Rd,K (= ANDI Rd,~K)"),
        "asr" => d("Arithmetic Shift Right", "Rd(n) <- Rd(n+1), bit 7 kept, C <- Rd(0)", "Z C N V S", ""),
        "bclr" => d("Bit Clear in SREG", "SREG(s) <- 0", "SREG(s)", "CLC CLZ CLN CLV CLS CLH CLT CLI"),
        "bld" => d("Bit Load from T to Register", "Rd(b) <- T", "-", ""),
        "brbc" => d("Branch if Bit in SREG is Cleared", "if SREG(s) = 0 then PC <- PC + k + 1", "-", "BRNE BRCC BRSH BRPL BRVC BRGE BRHC BRTC BRID"),
        "brbs" => d("Branch if Bit in SREG is Set", "if SREG(s) = 1 then PC <- PC + k + 1", "-", "BREQ BRCS BRLO BRMI BRVS BRLT BRHS BRTS BRIE"),
        "break" => d("Break (stops in the debugger)", "On-chip debug break", "-", ""),
        "bset" => d("Bit Set in SREG", "SREG(s) <- 1", "SREG(s)", "SEC SEZ SEN SEV SES SEH SET SEI"),
        "bst" => d("Bit Store from Register to T", "T <- Rd(b)", "T", ""),
        "call" => d("Long Call to a Subroutine", "STACK <- PC + 2, PC <- k", "-", ""),
        "cbi" => d("Clear Bit in I/O Register", "I/O(A, b) <- 0", "-", ""),
        "com" => d("One's Complement", "Rd <- 0xFF - Rd", "Z C N V S", ""),
        "cp" => d("Compare", "Rd - Rr (result discarded)", "Z C N V S H", ""),
        "cpc" => d("Compare with Carry", "Rd - Rr - C (result discarded)", "Z C N V S H", ""),
        "cpi" => d("Compare with Immediate", "Rd - K (result discarded)", "Z C N V S H", ""),
        "cpse" => d("Compare, Skip if Equal", "if Rd = Rr then skip the next instruction", "-", ""),
        "dec" => d("Decrement", "Rd <- Rd - 1", "Z N V S", ""),
        "des" => d("Data Encryption Standard round", "DES round K on R0..R15", "-", ""),
        "eicall" => d("Extended Indirect Call", "STACK <- PC + 1, PC <- EIND:Z", "-", ""),
        "eijmp" => d("Extended Indirect Jump", "PC <- EIND:Z", "-", ""),
        "elpm" => d("Extended Load Program Memory", "Rd <- (RAMPZ:Z)", "-", ""),
        "eor" => d("Exclusive OR", "Rd <- Rd ^ Rr", "Z N V S", "CLR Rd (= EOR Rd,Rd)"),
        "fmul" => d("Fractional Multiply Unsigned", "R1:R0 <- (Rd x Rr) << 1", "Z C", ""),
        "fmuls" => d("Fractional Multiply Signed", "R1:R0 <- (Rd x Rr) << 1", "Z C", ""),
        "fmulsu" => d("Fractional Multiply Signed with Unsigned", "R1:R0 <- (Rd x Rr) << 1", "Z C", ""),
        "icall" => d("Indirect Call to (Z)", "STACK <- PC + 1, PC <- Z", "-", ""),
        "ijmp" => d("Indirect Jump to (Z)", "PC <- Z", "-", ""),
        "in" => d("Load an I/O Location to Register", "Rd <- I/O(A)", "-", ""),
        "inc" => d("Increment", "Rd <- Rd + 1", "Z N V S", ""),
        "jmp" => d("Jump", "PC <- k", "-", ""),
        "lac" => d("Load and Clear", "(Z) <- Rd & ~(Z), Rd <- (Z)", "-", ""),
        "las" => d("Load and Set", "(Z) <- Rd | (Z), Rd <- (Z)", "-", ""),
        "lat" => d("Load and Toggle", "(Z) <- Rd ^ (Z), Rd <- (Z)", "-", ""),
        "ld" => d("Load Indirect from Data Space", "Rd <- (X/Y/Z), optional post-increment / pre-decrement", "-", ""),
        "ldd" => d("Load Indirect with Displacement", "Rd <- (Y/Z + q)", "-", ""),
        "ldi" => d("Load Immediate", "Rd <- K (r16..r31)", "-", "SER Rd (= LDI Rd,0xFF)"),
        "lds" => d("Load Direct from Data Space", "Rd <- (k)", "-", ""),
        "lpm" => d("Load Program Memory", "Rd <- (Z), optional post-increment", "-", ""),
        "lsr" => d("Logical Shift Right", "Rd(n) <- Rd(n+1), Rd(7) <- 0, C <- Rd(0)", "Z C N V S", ""),
        "mov" => d("Copy Register", "Rd <- Rr", "-", ""),
        "movw" => d("Copy Register Word", "Rd+1:Rd <- Rr+1:Rr", "-", ""),
        "mul" => d("Multiply Unsigned", "R1:R0 <- Rd x Rr", "Z C", ""),
        "muls" => d("Multiply Signed", "R1:R0 <- Rd x Rr", "Z C", ""),
        "mulsu" => d("Multiply Signed with Unsigned", "R1:R0 <- Rd x Rr", "Z C", ""),
        "neg" => d("Two's Complement", "Rd <- 0x00 - Rd", "Z C N V S H", ""),
        "nop" => d("No Operation", "-", "-", ""),
        "or" => d("Logical OR", "Rd <- Rd | Rr", "Z N V S", ""),
        "ori" => d("Logical OR with Immediate", "Rd <- Rd | K", "Z N V S", "SBR Rd,K"),
        "out" => d("Store Register to I/O Location", "I/O(A) <- Rr", "-", ""),
        "pop" => d("Pop Register from Stack", "Rd <- STACK", "-", ""),
        "push" => d("Push Register on Stack", "STACK <- Rr", "-", ""),
        "rcall" => d("Relative Call to Subroutine", "STACK <- PC + 1, PC <- PC + k + 1", "-", ""),
        "ret" => d("Return from Subroutine", "PC <- STACK", "-", ""),
        "reti" => d("Return from Interrupt", "PC <- STACK, I <- 1", "I", ""),
        "rjmp" => d("Relative Jump", "PC <- PC + k + 1", "-", ""),
        "ror" => d("Rotate Right through Carry", "Rd(7) <- C, Rd(n) <- Rd(n+1), C <- Rd(0)", "Z C N V S", ""),
        "sbc" => d("Subtract with Carry", "Rd <- Rd - Rr - C", "Z C N V S H", ""),
        "sbci" => d("Subtract Immediate with Carry", "Rd <- Rd - K - C", "Z C N V S H", ""),
        "sbi" => d("Set Bit in I/O Register", "I/O(A, b) <- 1", "-", ""),
        "sbic" => d("Skip if Bit in I/O Register is Cleared", "if I/O(A, b) = 0 then skip", "-", ""),
        "sbis" => d("Skip if Bit in I/O Register is Set", "if I/O(A, b) = 1 then skip", "-", ""),
        "sbiw" => d("Subtract Immediate from Word", "Rd+1:Rd <- Rd+1:Rd - K", "Z C N V S", ""),
        "sbrc" => d("Skip if Bit in Register is Cleared", "if Rr(b) = 0 then skip", "-", ""),
        "sbrs" => d("Skip if Bit in Register is Set", "if Rr(b) = 1 then skip", "-", ""),
        "sleep" => d("Sleep", "Enter the sleep mode selected in SMCR (if SE = 1)", "-", ""),
        "spm" => d("Store Program Memory", "(Z) <- R1:R0 (self-programming)", "-", ""),
        "st" => d("Store Indirect to Data Space", "(X/Y/Z) <- Rr, optional post-increment / pre-decrement", "-", ""),
        "std" => d("Store Indirect with Displacement", "(Y/Z + q) <- Rr", "-", ""),
        "sts" => d("Store Direct to Data Space", "(k) <- Rr", "-", ""),
        "sub" => d("Subtract without Carry", "Rd <- Rd - Rr", "Z C N V S H", ""),
        "subi" => d("Subtract Immediate", "Rd <- Rd - K", "Z C N V S H", ""),
        "swap" => d("Swap Nibbles", "Rd(7:4) <-> Rd(3:0)", "-", ""),
        "wdr" => d("Watchdog Reset", "Restart the watchdog timer", "-", ""),
        "xch" => d("Exchange", "(Z) <-> Rd", "-", ""),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_instruction_is_documented() {
        for def in super::super::isa::insns() {
            assert!(super::insn_doc(def.name).is_some(), "missing reference text for {}", def.name);
        }
    }
}
