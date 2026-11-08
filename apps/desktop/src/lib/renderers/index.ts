// Built-in Tier 0 renderers for the "web" target.
//
// The *decision* of which renderer to use is made in Rust (nmesh-contracts::resolve_renderer,
// surfaced as `body.renderer`); this file only maps renderer ids to components.

import type { Component } from "svelte";
import Card from "./Card.svelte";
import Form from "./Form.svelte";
import Location from "./Location.svelte";
import Table from "./Table.svelte";

export const BUILTIN: Record<string, Component<any>> = {
  "nmesh.card": Card,
  "nmesh.table": Table,
  "nmesh.form": Form,
  "nmesh.location": Location,
};
