// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { BindInfo } from "./BindInfo";
import type { HostAddress } from "./HostAddress";
import type { HostKind } from "./HostKind";

export type Host = { kind: HostKind, bindings: { [key: number]: BindInfo }, addresses: Array<HostAddress>, primary: HostAddress | null, };