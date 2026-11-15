/*
 * The hypercall of the 9h machine (rucc-plan9, plan 10.5).
 * uvlong h9call(uvlong fn, uvlong a1, uvlong a2, uvlong a3, uvlong a4)
 * The first argument is in RARG, and the others are on the
 * stack. The OUTB to port 0x509 stops the processor in the
 * monitor, which reads the registers and puts the result in
 * AX. The kernel saves no register of the caller, so BX, SI
 * and DI can change.
 */
TEXT h9call(SB), 1, $-4
	MOVQ	RARG, AX
	MOVQ	a1+8(FP), BX
	MOVQ	a2+16(FP), CX
	MOVQ	a3+24(FP), SI
	MOVQ	a4+32(FP), DI
	MOVL	$0x509, DX
	OUTB
	RET
