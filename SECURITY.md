# Security policy

This crate runs WebAssembly modules that you did not write.
It is the boundary between your proxy and a plugin, so a defect in it can let a plugin reach what your proxy holds.

## Reporting a vulnerability

Report a vulnerability through the private security advisory form of this repository, under the Security tab.
Do not open a public issue for one.

Include the version or the commit you tested, the module or the source of the guest that shows the problem, the host functions or the callbacks it calls, and what the guest reached that it should not have.
A guest you can share, even a small one, shortens the work more than a description does.

You can expect an answer within one week.
The answer says whether the report is in scope, and it gives the next step.

## Scope

A vulnerability of this crate is a way for a guest to reach something the host did not lend it.
These are in scope.

- A guest reads or writes the state of another guest, or of another request of the same guest.
- A guest reaches a queue, a metric, or a shared key of a virtual machine that is not its own.
- A guest makes the host read or write memory outside the guest's own memory.
- A guest makes the host spend memory or time without a limit, through a call that the documented limits say is bounded.
- The crate gives a guest a value that no service of yours supplied.

These are not in scope.

- A defect of wasmtime, which belongs to that project. Report it there.
- A service of yours that gives a guest more than you meant to. The crate passes what your implementation answers.
- An environment variable that you set on the guest. The guest reads every one of them.
- A guest that traps, or that spends its own limits. A trap is a normal end for a call, and your embedder builds a new guest.
- A denial of service that comes from the traffic you send to the plugin rather than from the plugin.

## Supported versions

The crate is not published yet, so no version carries a security promise.
The first release adds a table here that names the versions that receive fixes.

## What this crate defends

The crate treats the guest as untrusted and your embedder as trusted.
Every value a guest sends arrives as bytes of guest memory, and the crate reads them through bounds checks before any service of yours sees them.
Every identifier a guest names is checked against the identifiers that guest obtained, so a small number that another virtual machine could guess reaches nothing.
The crate uses no `unsafe` code of its own.

## What your embedder must answer

The crate serves the ABI, and four duties stay with you.

- Separate your tenants. One virtual machine identifier separates the shared data, the queues, and the metrics, so give one identifier to each tenant.
- Bound what your services keep. The crate bounds what a guest sends and what it holds. What your store keeps across guests is yours to bound.
- Recover a guest that traps. A trap poisons the instance, and the requests that wait on a callout of it wait forever until you end them.
- Watch the rate of the rebuilds. A guest that traps on every request costs a rebuild on every request, and the crate counts the builds that followed a poisoned guest so you can see it.

## The limits you can change

The module documentation of the ABI has a table of every limit, its default, and the method that changes it.
Two types carry them.
`Limits` carries what one instance and one guest may spend, and `InMemoryStore` carries what the reference store keeps.
A limit set to `None` is removed, and a guest then meets no bound of the crate on that call.
