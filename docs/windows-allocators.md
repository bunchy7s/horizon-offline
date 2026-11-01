# Windows allocator experiments

The normal client already uses mimalloc through `horizon_threads::Allocator`,
including its startup configuration and memory-release behavior. It does not
request Windows Segment Heap. An off-by-default Cargo feature allows a separate
experiment with the Win32 heap used by native libraries:

| Feature | Effect on Windows |
| --- | --- |
| `windows-segment-heap` | Embeds a manifest requesting Segment Heap for Win32 heap APIs in this package's executables. |
| `windows-mimalloc` | Compatibility no-op for earlier experiment commands; the client already uses mimalloc without this feature. |

Neither feature changes non-Windows builds relative to the current main branch.
Segment Heap is a build option, not a mod or runtime setting; changing it requires
rebuilding and restarting the client. It does not replace or reconfigure the Rust
allocator. Native libraries can still use Windows heap APIs even though Rust uses
mimalloc. No frame-rate improvement is guaranteed.

```sh
# Current mimalloc baseline, then add Segment Heap. Keep the same profile.
cargo build --profile fast -p horizon-client --bin horizon-client
cargo build --profile fast -p horizon-client --bin horizon-client --features windows-segment-heap
# Older commands still build, but these repeat the two variants above.
cargo build --profile fast -p horizon-client --bin horizon-client --features windows-mimalloc
cargo build --profile fast -p horizon-client --bin horizon-client --features windows-segment-heap,windows-mimalloc
```

Copy each executable to a separate experiment directory before building the next
variant, or use a separate `CARGO_TARGET_DIR`. Keep prepared resources and worlds
outside version control. Use disposable copies of a world and isolated data
directories for tests, following [AGENTS.md](../AGENTS.md).

Segment Heap is supported starting with Windows 10 version 2004 (build 19041),
according to Microsoft's [application manifest documentation](https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests#heaptype).
The manifest leaves the optional architecture unspecified and keeps the usual `asInvoker`
execution level. The existing icon and version resources are compiled together
with it through the existing Windows-host resource build step. Cross-compiling
that manifest from a non-Windows host is not supported by this build step.
Builds on earlier Windows versions have not been tested.

## Validate the executable

On Windows, extract the manifest from a built executable using the Windows SDK's
`mt.exe` (PowerShell syntax):

```powershell
mt.exe '-inputresource:target/fast/horizon-client.exe;#1' '-out:client.manifest'
mt.exe -manifest client.manifest -validate_manifest
```

Check that the feature build contains `SegmentHeap` in the
`http://schemas.microsoft.com/SMI/2020/WindowsSettings` namespace. The baseline
must not contain this element. Confirm Explorer still shows Horizon's icon for
both builds. Inspecting the final executable matters: a valid source XML file
alone does not prove that the linker embedded it.

## Measure separately

Use the same commit, compiler, profile, settings, driver configuration, world
copy, warm-up period and flight route for both variants. Alternate their
order across repeated sessions. Capture individual frame times, their p99 and
maximum, memory use and average FPS. Record game FPS limits, the display refresh
rate and driver-level VSync: a driver override can cap FPS even when game VSync
is disabled.

Compare the current mimalloc baseline with that same build plus Segment Heap.
The compatibility feature cannot measure system allocation versus mimalloc.
Results from older system-allocator builds, or builds that also change meshing
or LOD retirement, cannot establish the benefit of Segment Heap on current main.
