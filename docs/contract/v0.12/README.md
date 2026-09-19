# Contract 0.12.0: exact decimals and nominal quantities

This profile adds exact scalar schemas without changing existing graph, assertion, context or proof semantics. Versions 0.1–0.11 remain accepted for their existing features. A Commit or CommitBatch containing either new scalar schema requires version 0.12.0; the runtime checks node and edge declarations before any writes.

`ScalarType::Decimal` is JSON `"decimal"`. A property is a canonical decimal string: `"0.3"`, not the binary64 number `0.3`, `"0.30"` or `"3e-1"`. Values contain at most 18 coefficient digits and 18 fractional places; zero is `"0"`. Portable parsing can normalize bounded decimal/exponent input, but wire deserialization requires canonical spelling. Arithmetic reports overflow, excessive scale, division by zero and nonterminating division; it never rounds or falls back to Float.

`ScalarType::Quantity(UnitDescriptor)` is JSON `{"quantity":{"dimension_id":"length","unit_id":"metre","revision":"1"}}`. A quantity property is `{"amount":"0.3","unit":{"dimension_id":"length","unit_id":"metre","revision":"1"}}`. The entire descriptor must equal its schema declaration. Unit spelling does not establish identity, conversion authority or compatibility with geometry evidence.

Portable Decimal and Quantity APIs implement exact bounded arithmetic and explicit positive rational conversions between nominal descriptors sharing a dimension identifier. Offset conversions, general dimension products, graph-backed conversion authorization and quantity vectors remain open. The source frontend may evaluate pure literal arithmetic with the same checked modules; no remote capability follows from an arithmetic result. See [numeric limits and evidence](../../EXACT_QUANTITIES.md).

Existing scalar serialization and old schema digests are unchanged. Nullable property behavior is unchanged. Query, capsule transport and graph algebra preserve declared schemas and exact values. Malformed/noncanonical values and mismatched descriptors fail schema validation atomically, including programs with an earlier valid commit. Source and runtime tests exercise node and edge properties through single and batch commits and capsule receipt.

This is a bounded scalar profile, not completion of the language's entire type algebra or dimensional reasoning requirements.
