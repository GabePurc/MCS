# RVC: every compressed instruction explicitly (c.* mnemonics), 2-byte aligned 32-bit instructions.
.option rvc

_start:
    PROLOGUE
rvc_start:
    # ---- c.li / c.addi / c.nop / c.mv / c.add ----
    c.li a0, -5
    c.addi a0, 3
    c.nop
    REC a0                        # 0xfffffffe
    c.li a1, 31
    c.addi a1, -32
    REC a1                        # 0xffffffff
    c.mv a2, a0
    c.add a2, a1                  # 0xfffffffe + 0xffffffff
    REC a2                        # 0xfffffffd
    c.li a3, 0
    c.mv a3, a1
    REC a3                        # 0xffffffff
    # ---- c.lui ----
    c.lui a2, 31
    REC a2                        # 0x0001f000
    c.lui a3, 0xfffe1
    REC a3                        # 0xfffe1000
    # ---- c.addi16sp / c.addi4spn ----
    mv s0, sp
    c.addi16sp sp, -64
    sub t0, s0, sp
    REC t0                        # 64
    c.addi16sp sp, 496
    sub t0, sp, s0
    REC t0                        # 432
    c.addi16sp sp, -512
    sub t0, s0, sp
    REC t0                        # 80
    c.addi4spn a4, sp, 1020
    sub t0, a4, sp
    REC t0                        # 1020
    c.addi4spn a5, sp, 4
    sub t0, a5, sp
    REC t0                        # 4
    mv sp, s0
    # ---- c.sw / c.lw / c.swsp / c.lwsp ----
    lui s0, 0x3fc80
    addi s0, s0, 0x400
    li a0, 0x01020304
    c.sw a0, 4(s0)
    c.sw a0, 124(s0)
    c.lw a1, 4(s0)
    REC a1                        # 0x01020304
    c.lw a2, 124(s0)
    REC a2                        # 0x01020304
    c.swsp a0, 8(sp)
    c.swsp a0, 252(sp)
    c.lwsp a3, 8(sp)
    REC a3                        # 0x01020304
    c.lwsp a4, 252(sp)
    REC a4                        # 0x01020304
    lw a5, 4(s0)
    c.lw a5, 0(s0)                # untouched memory (zero) overwrites a5
    REC a5                        # 0 (DRAM at 0x3FC80400 is zeroed on reset)
    # ---- shifts / logic ----
    li a0, 0xf0f0f0f0
    c.srli a0, 4
    REC a0                        # 0x0f0f0f0f
    li a0, 0x80000000
    c.srai a0, 31
    REC a0                        # 0xffffffff
    li a0, 0x80000000
    c.srli a0, 31
    REC a0                        # 1
    li a0, 0x12345678
    c.andi a0, -16
    REC a0                        # 0x12345670
    c.andi a0, 15
    REC a0                        # 0
    li a0, 0xff00ff00
    li a1, 0x0ff00ff0
    c.sub a0, a1                  # 0xff00ff00 - 0x0ff00ff0
    REC a0                        # 0xef10ef10
    li a0, 0xff00ff00
    c.xor a0, a1
    REC a0                        # 0xf0f0f0f0
    li a0, 0xff00ff00
    c.or a0, a1
    REC a0                        # 0xfff0fff0
    li a0, 0xff00ff00
    c.and a0, a1
    REC a0                        # 0x0f000f00
    li a0, 1
    c.slli a0, 31
    REC a0                        # 0x80000000
    li a1, 0xabcd
    c.slli a1, 8
    REC a1                        # 0x00abcd00
    # ---- jumps / branches ----
    c.j 1f
    REC x0                        # skipped
1:
    c.jal csub1                   # ra = return address
rj1:
    REC a0                        # 21
    la t0, rj1
    sub t0, ra, t0
    REC t0                        # 0 (ra == rj1, the instruction after c.jal)
    la a1, csub2
    c.jalr a1
    REC a0                        # 17
    c.li a0, 0
    c.beqz a0, 2f                 # taken
    c.li a0, 1
2:
    REC a0                        # 0
    c.bnez a0, 3f                 # not taken
    c.li a0, 2
3:
    REC a0                        # 2
    c.bnez a0, 4f                 # taken
    c.li a0, 3
4:
    REC a0                        # 2
    c.li a1, 1
    c.beqz a1, 5f                 # not taken
    c.li a1, 9
5:
    REC a1                        # 9
    la a0, csub3
    c.jr a0                       # csub3 jumps back to rvc_mix
rvc_back:
    # ---- 32-bit instructions on 2-byte boundaries ----
.balign 4
rvc_mix:
    c.nop
.option push
.option norvc
    auipc t0, 0                   # pc % 4 == 2
    andi t0, t0, 3
    REC t0                        # 2
.option pop
    c.nop
.option push
.option norvc
    jal ra, rvm1                  # 4-byte jal at pc % 4 == 0 mod, target 2-aligned
rvm0:
    beq x0, x0, rvm2              # 4-byte branch
.option pop
    c.li a0, 30
    REC a0                        # skipped
rvm1:
    c.nop
    ret
rvm2:
    la t1, rvm0
    sub t1, ra, t1                # ra == rvm0 (instruction after the jal)
    REC t1                        # 0
rvc_end:
done:
    c.ebreak

csub1:
    c.li a0, 21
    c.jr ra
csub2:
    c.li a0, 17
    c.jr ra
csub3:
    j rvc_mix
