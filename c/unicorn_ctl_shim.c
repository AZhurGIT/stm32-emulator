/*
 * Call Unicorn uc_ctl(UC_CTL_TB_FLUSH) from C so the x86_64 variadic ABI is correct.
 * Rust cannot call uc_ctl(...) safely: the callee expects AL=0 on SysV amd64 for
 * variadic functions; a plain extern "C" fn(uc, u32) call from Rust does not
 * guarantee that and may crash inside va_start (observed as SIGSEGV right after IRQ).
 */
#include <stdint.h>

typedef void uc_engine;

/* uc_err from unicorn.h — keep in sync with unicorn_engine::unicorn_const::uc_error */
typedef enum {
    UC_ERR_OK = 0,
} uc_err;

extern uc_err uc_ctl(uc_engine *uc, uint32_t control, ...);

/* unicorn.h: UC_CTL_WRITE(UC_CTL_TB_FLUSH, 0) */
#define UC_CTL_IO_WRITE (1u)
#define UC_CTL(type, nr, rw) \
    ((uint32_t)(type) | ((uint32_t)(nr) << 26) | ((uint32_t)(rw) << 30))
#define UC_CTL_WRITE(type, nr) UC_CTL(type, nr, UC_CTL_IO_WRITE)
#define UC_CTL_TB_FLUSH (10u)

uc_err stm32_emu_uc_ctl_tb_flush(void *uc)
{
    return uc_ctl((uc_engine *)uc, UC_CTL_WRITE(UC_CTL_TB_FLUSH, 0));
}
