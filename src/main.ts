import { createApp } from 'vue';
import App from './App.vue';
import ScanVisitor from './ScanVisitor.vue';
import './style.css';
const scanRoute = window.location.pathname.startsWith('/scan/');
const scanKey = /^\/scan\/([a-f0-9]{64})$/.exec(window.location.pathname)?.[1] || '';
createApp(scanRoute ? ScanVisitor : App, scanRoute ? { scanKey } : undefined).mount('#app');
