import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import './index.css';
import { soulbrowserAPI } from './api/soulbrowser';
import { apiClient } from './api/client';

const DEFAULT_DEV_TOKEN = 'soulbrowser-dev-token';

const bootstrapServeToken = () => {
  if (typeof window === 'undefined' || typeof localStorage === 'undefined') {
    return;
  }

  const envToken = import.meta.env.VITE_SERVE_TOKEN?.trim();
  const token = envToken && envToken.length > 0 ? envToken : DEFAULT_DEV_TOKEN;
  localStorage.setItem('serve_token', token);
  localStorage.setItem('auth_token', token);
};

bootstrapServeToken();

const applyDefaultBaseUrl = () => {
  if (typeof window === 'undefined') {
    return;
  }
  const origin = window.location.origin;
  soulbrowserAPI.setBaseUrl(origin);
  apiClient.setBaseUrl(origin);
};

applyDefaultBaseUrl();

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
