import { mount } from 'svelte';
import '@xterm/xterm/css/xterm.css';
import './app.css';
import App from './App.svelte';
import { apply, theme } from './lib/theme.svelte';

apply(theme.value);

export default mount(App, { target: document.getElementById('app')! });
