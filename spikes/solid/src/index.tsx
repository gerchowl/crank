import { render } from "solid-js/web";
import "./theme/tokens.css";
import "./theme/palette.css";
import "./styles/app.css";
import { App } from "./App";

const root = document.getElementById("root");
if (root) render(() => <App />, root);
