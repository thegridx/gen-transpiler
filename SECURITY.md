# Security policy

gen-transpiler exists to run untrusted RISC-V programs safely, so we treat any way around its
sandbox as a security issue. Examples:

- transpiled code that reads or writes memory outside its own regions;
- transpiled code that jumps outside its own code;
- a program that runs without paying for the gas it uses;
- a RISC-V input ELF that makes the transpiler crash, hang or produce wrong code;
- transpiler output that makes the loader or the runner crash or misbehave.

## Reporting a vulnerability

Do not open a public issue. Report it privately through GitHub: open the repository's
**Security** tab and choose **Report a vulnerability**.

Include the input program or a way to build it, the target architecture, and what you observed.
We will confirm receipt and keep you informed until the issue is fixed.

## Scope

The sandbox is a first layer of protection. A host that runs untrusted code should also isolate
it, for example in a separate process with restricted privileges. Out of scope:

- code built with `--trusted` (`TranspilationMode::Trusted`), which has no sandbox;
- native artifacts that were modified after the transpiler produced them. A native artifact
  is code, so a host must load only artifacts it transpiled itself or whose integrity it
  verified. The loader's checks catch mistakes, not tampering.

The [Security model](README.md#security-model) section of the README describes these limits.
