/**
 * WebSocket client for real-time updates.
 */

import type { WsClientMessage, WsServerMessage } from "./types";

export type MessageHandler = (message: WsServerMessage) => void;

interface WebSocketClient {
  send: (message: WsClientMessage) => void;
  close: () => void;
}

/**
 * Connect to the WebSocket endpoint with automatic reconnection.
 */
export function connectWebSocket(
  onMessage: MessageHandler,
  options?: {
    onOpen?: () => void;
    onClose?: () => void;
    onError?: (error: Event) => void;
    reconnect?: boolean;
    reconnectDelay?: number;
  }
): WebSocketClient {
  const {
    onOpen,
    onClose,
    onError,
    reconnect = true,
    reconnectDelay = 3000,
  } = options ?? {};

  let ws: WebSocket | null = null;
  let reconnectTimeout: number | null = null;
  let isClosing = false;

  function getWsUrl(): string {
    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    return `${protocol}//${window.location.host}/api/ws`;
  }

  function connect() {
    if (isClosing) return;

    ws = new WebSocket(getWsUrl());

    ws.onopen = () => {
      console.log("[WS] Connected");
      onOpen?.();
      // Start ping interval to keep connection alive
      startPing();
    };

    ws.onmessage = (event) => {
      try {
        const message = JSON.parse(event.data as string) as WsServerMessage;
        onMessage(message);
      } catch (e) {
        console.error("[WS] Failed to parse message:", e);
      }
    };

    ws.onclose = () => {
      console.log("[WS] Disconnected");
      stopPing();
      onClose?.();

      // Attempt reconnect if not intentionally closed
      if (reconnect && !isClosing) {
        console.log(`[WS] Reconnecting in ${reconnectDelay}ms...`);
        reconnectTimeout = window.setTimeout(connect, reconnectDelay);
      }
    };

    ws.onerror = (error) => {
      console.error("[WS] Error:", error);
      onError?.(error);
    };
  }

  // Ping to keep connection alive
  let pingInterval: number | null = null;

  function startPing() {
    stopPing();
    pingInterval = window.setInterval(() => {
      if (ws?.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: "ping" }));
      }
    }, 30000); // Ping every 30 seconds
  }

  function stopPing() {
    if (pingInterval !== null) {
      clearInterval(pingInterval);
      pingInterval = null;
    }
  }

  // Initial connection
  connect();

  return {
    send: (message: WsClientMessage) => {
      if (ws?.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify(message));
      } else {
        console.warn("[WS] Cannot send, not connected");
      }
    },
    close: () => {
      isClosing = true;
      stopPing();
      if (reconnectTimeout !== null) {
        clearTimeout(reconnectTimeout);
      }
      ws?.close();
    },
  };
}
