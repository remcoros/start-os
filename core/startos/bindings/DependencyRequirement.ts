// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.

export type DependencyRequirement = { "kind": "running", id: string, healthChecks: string[], versionSpec: string, url: string, } | { "kind": "exists", id: string, versionSpec: string, url: string, };