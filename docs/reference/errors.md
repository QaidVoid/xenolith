# Diagnostics

Every failure the tool can report, what it exits with, and what to do about it.
Each code below is produced by a test in `crates/xenolith/tests/exit_codes.rs`,
so a page that drifts from the binary fails the gate.

## The shape of a diagnostic

A failure is a message naming what was being done, and under it the cause. What
the tool actually prints for a missing file:

```text
Error:   x reading /nonexistent.xex
  ╰─▶ No such file or directory (os error 2)
```

The first line names the operation and the path. The second is the cause, passed
through rather than paraphrased. Where there is a specific thing to try, a `help`
line follows it.

Nothing is printed to standard output on a failure, so a caller can read the exit
code and know whether the output is worth reading.

## Exit codes

| code | meaning |
|---|---|
| 0 | the command did what was asked |
| 1 | the command could not do what was asked, and said why |
| 2 | the arguments were not a command this tool has |

**0** covers `--help` and `--version` as well as a command that ran.

**1** is every failure with a reason: a file that will not read, a container that
will not decode, an address that is not four byte aligned, a part size of zero, a
root that is not a number, and a lift that found nothing to lift.

**2** is the argument parser refusing before anything is read: no subcommand at
all, or one that does not exist. This is the same code the parser uses for a
missing required argument.

## The failures worth knowing

**A key is needed and none was found.** An encrypted container cannot be decoded
without one. `inspect` reads the header and does not need a key; `analyze`,
`disasm` and `lift` decode the image and do. The message names the environment
variable, the file path it looked in, and `--raw` with an already decoded image,
which needs no key at all.

**An address is not in an executable section.** `disasm` refuses rather than
reading data as code. `--allow-data` disassembles it anyway, for the case where
the reading is the point.

**An address is not four byte aligned.** Instructions are four bytes, so a
shifted start produces plausible looking and entirely wrong output. The message
says so rather than printing it.

**Nothing was discovered to lift.** Analysis found no functions at all, which
means the image is not what the caller thinks it is.

**A part size of zero.** A unit has to hold at least one function, and a budget
of zero would write one unit per byte.

## Traces

Three environment variables make a run report rather than stop. Each is a
diagnostic, and each makes the run after it describe the answer it was given
rather than the title.

| variable | what it reports |
|---|---|
| `XENOLITH_TRACE_UNLIFTED` | every function a built title called that was never translated, carrying on past it |
| `XENOLITH_TRACE_DISPATCH` | every address a built title dispatched to that no function answers, carrying on past it |
| `XENOLITH_TRACE_IMPORTS` | every import a built title reaches, answering each with nothing |

See [Environment](/reference/environment) for the rest.
