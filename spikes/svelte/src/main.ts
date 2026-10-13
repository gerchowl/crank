import { mount } from "svelte";
import App from "./App.svelte";
import "./styles/tokens.css";
import "./styles/base.css";

const target = document.getElementById("app");
if (!target) throw new Error("#app missing");
export default mount(App, { target });
import "uplot/dist/uPlot.min.css";
