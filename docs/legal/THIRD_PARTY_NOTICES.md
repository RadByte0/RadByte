# Third-party notices

This file records third-party code used by RadByte. It does not change the license of RadByte's own code.

## fastnbt 2.6.3

- Project: [owengage/fastnbt](https://github.com/owengage/fastnbt)
- Source revision used to identify the published crate: [`d71c04b2e73d1365c0d677e1f42cf573ccc2b281`](https://github.com/owengage/fastnbt/tree/d71c04b2e73d1365c0d677e1f42cf573ccc2b281/fastnbt)
- Crate license expression: `MIT OR Apache-2.0`; the upstream license file at that revision contains the MIT license below.
- Use in RadByte: development-only test dependency. `RadCore/src/protocol/nbt.rs` checks network NBT encoding and decoding against FastNBT. No FastNBT implementation source is vendored or copied into RadByte.

### MIT License

Copyright (c) 2020 Owen Gage

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
