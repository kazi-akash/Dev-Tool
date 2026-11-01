import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "@/app/App";
import { QuickStatusApp } from "@/features/quick/QuickStatusApp";
import "./index.css";

// The tray popup is the same bundle on a different route. It skips the app shell and settings
// provider so it stays tiny and calls only the two commands its window may use.
const popup = window.location.hash.startsWith("#/quick-status");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(<React.StrictMode>{popup ? <QuickStatusApp /> : <App />}</React.StrictMode>);
