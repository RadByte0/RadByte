# Item component port

1. Select a specific upstream implementation, pin its revision, and review its license and provenance. Exclude generated game data whose redistribution rights are unclear.
2. Compare the chosen component's wire format with protocol 777 and identify its storage and inventory paths in RadByte.
3. Translate a bounded, useful component into Rust, preserving upstream attribution and documenting the source and changes.
4. Verify wire interoperability, invalid input handling, inventory behavior, and persistence before extending support to additional components.
5. Record remaining limitations; avoid claiming complete vanilla compatibility.

## Initial source review

- Candidate: `ezchr/go-mcjava`, revision `985587f5b3b732980fd5d3037f7d0bb01c958857`, root license MIT; copyright 2026 ezchr. The root license also preserves a separate notice for `internal/cfb8`.
- `item/item.go` distinguishes server-to-client component encoding from the client-to-server form, which prefixes component values with their length. Review the packet direction before adapting it.
- The generated registry reports `max_stack_size = 1` and `repair_cost = 19`; RadByte currently uses 15 for repair cost. Treat the discrepancy as unresolved until independently validated for the actual target protocol and address encode, decode, and persistence together. The registry is generated from game data; do not copy it without examining its provenance.
- Preserve the MIT copyright and license notice with any translated code. MIT permits redistribution subject to its notice conditions; it does not by itself establish rights in generated game data.
