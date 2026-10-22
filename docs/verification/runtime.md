# The runtime oracle

**What only it catches.** A service that answers wrongly while the emitted code
still links. Everything else here compares strings, compiles them, or runs one
function. This is the only oracle that runs the runtime itself.

**Input.** The runtime, compiled with `-Wall -Wextra -Werror -pedantic` beside a
small program that drives it. What it checks is what the code does rather than
what it says.

**Pass condition.** Every scenario passes, and the whole thing is built twice more
under AddressSanitizer with UndefinedBehaviorSanitizer and under ThreadSanitizer
and comes back clean.

**Command.**

```sh
cargo test --release -p xenolith-lift --test runtime
```

It needs a C compiler and nothing else, so it runs in `scripts/check` rather than
in `scripts/check-oracles`.

**What the sanitizers add.** Address finds what reads or writes outside what it
owns, which is the whole class a test cannot catch here: the runtime indexes a
base pointer with a guest address and has no check anywhere. Thread finds what two
threads did to one thing at once, which is what the wait queue and the
thread-local slots are made of.

**The scenarios.** Guest memory and the heap, the processor block a thread reaches
through `r13`, waits and timeouts, critical sections, thread-local slots, a title's
files, the graphics command stream, the vertex arena, the imports, and the shape
of the interface itself.

**Each is written to fail when its service is broken.** That was checked by
breaking them: a shared thread-local array makes the thread-local scenario fail,
removing the `r13` assignment makes the processor-block one fail, and a wait with
a timeout hangs the suite rather than passing when there is no timed wait.

**Limit.** A compiler without the sanitizers is not a failure and is not a pass
either: the test reports itself ignored and names what is missing, and
`scripts/check-oracles` runs it so that an ignore there is a failure.

What this cannot check is whether a service answers what a console would have. It
checks that each one answers what this project says it does. A service that is
internally consistent and wrong about the hardware is invisible to it, which is
why the host assumptions a run reports are stated rather than hidden.
