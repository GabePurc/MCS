/* DWARF line-table fixture used by tests/formats/elf.test.ts.
 * Regenerate from this directory with:
 *   clang -target i386-unknown-linux-gnu -g -gdwarf-5 -O0 -fno-asynchronous-unwind-tables \
 *     -fdebug-prefix-map=$PWD=/fixture -c line_fixture.c -o line_fixture.dwarf5.o
 * and the same command with -gdwarf-4 writing line_fixture.dwarf4.o.
 * The tests assert the line numbers noted below: do not reflow this file. */
#include "line_fixture.h"

int counter = 3;

static int square(int x)
{
  return x * x; /* line 13 */
}

int accumulate(int n)
{
  int total = 0; /* line 18 */
  for (int i = 0; i < n; i++)
    total += square(i); /* line 20 */
  return total; /* line 21 */
}

int main(void)
{
  return twice(accumulate(counter)); /* line 26 */
}
