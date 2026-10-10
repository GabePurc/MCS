# TIMG0 general-purpose timer T0: XTAL / 40 = 1 MHz, alarm at 500 us, polled through INT_RAW.
_start:
    PROLOGUE
    li s0, TIMG0
    li t1, 500
    sw t1, 0x10(s0)                 # T0ALARMLO
    sw zero, 0x14(s0)
    sw zero, 0x18(s0)               # T0LOADLO / HI = 0
    sw zero, 0x1c(s0)
    sw zero, 0x20(s0)               # T0LOAD: counter = 0
    li t0, (1 << 31) | (1 << 30) | (40 << 13) | (1 << 9) | (1 << 10)
    sw t0, 0(s0)                    # EN, INCREASE, DIVIDER = 40, USE_XTAL, ALARM_EN
1:  lw t2, 0x74(s0)                 # INT_RAW_TIMERS
    andi t2, t2, 1
    beqz t2, 1b
    li t1, 1 << 31
    sw t1, 0x0c(s0)                 # T0UPDATE
    lw t3, 4(s0)                    # T0LO
    REC t3                          # [0] counter when the alarm was noticed
    lw t4, 0(s0)                    # T0CONFIG: ALARM_EN cleared by the alarm
    REC t4
    li t1, 1
    sw t1, 0x7c(s0)                 # INT_CLR
    lw t2, 0x74(s0)
    REC t2                          # [2] raw status after clearing = 0
    ebreak
