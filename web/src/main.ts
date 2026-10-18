// The page: the Svelte shell over the worker (`rpc.ts`) that runs the core and the engine.

import "./app.css";
import { mount } from "svelte";
import App from "./ui/App.svelte";
import { ask } from "./rpc";

mount(App, { target: document.getElementById("app")! });
// For a check driving the page.
(window as unknown as { ask: unknown }).ask = ask;
