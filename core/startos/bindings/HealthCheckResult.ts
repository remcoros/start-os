// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.

export type HealthCheckResult = { "result": "success" } | { "result": "disabled" } | { "result": "starting" } | { "result": "loading", message: string, } | { "result": "failure", error: string, };