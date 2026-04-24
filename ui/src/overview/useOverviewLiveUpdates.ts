import type { QueryClient } from '@tanstack/react-query';
import { useEffect, useEffectEvent, useState } from 'react';

import type { LiveUpdateStatus } from '../types';
import { invalidateOverviewQueries } from './queryClient';
import type { ToastMessage } from './shared';

export function useOverviewLiveUpdates({
  client,
  onToast
}: {
  client: QueryClient;
  onToast: (message: string, tone: ToastMessage['tone']) => void;
}) {
  const [liveUpdates, setLiveUpdates] = useState<LiveUpdateStatus>('connecting');
  const [streamKey, setStreamKey] = useState(0);

  const pushToast = useEffectEvent((message: string, tone: ToastMessage['tone']) => {
    onToast(message, tone);
  });
  const refreshQueries = useEffectEvent(() => {
    void invalidateOverviewQueries(client);
  });

  useEffect(() => {
    let active = true;
    let source: EventSource | null = null;
    let retries = 0;
    let reconnectTimer: number | null = null;
    let refreshTimer: number | null = null;
    let stormPauseNotified = false;
    let eventTimestamps: number[] = [];

    function flushRefresh() {
      refreshTimer = null;
      refreshQueries();
    }

    function pause(message: string) {
      source?.close();
      if (refreshTimer !== null) {
        window.clearTimeout(refreshTimer);
        refreshTimer = null;
      }

      setLiveUpdates('paused');
      if (!stormPauseNotified) {
        pushToast(message, 'error');
        stormPauseNotified = true;
      }
    }

    function handleServerEvent() {
      const now = Date.now();
      eventTimestamps = eventTimestamps.filter((timestamp) => now - timestamp < 2_000);
      eventTimestamps.push(now);

      if (eventTimestamps.length >= 12) {
        pause('Live updates paused due to an event storm.');
        return;
      }

      if (refreshTimer !== null) {
        return;
      }
      refreshTimer = window.setTimeout(flushRefresh, 250);
    }

    function connect() {
      if (!active) {
        return;
      }

      setLiveUpdates(retries === 0 ? 'connecting' : 'reconnecting');
      source = new EventSource('/events');

      source.onopen = () => {
        const wasReconnecting = retries > 0;
        retries = 0;
        stormPauseNotified = false;
        eventTimestamps = [];
        setLiveUpdates('live');
        if (wasReconnecting) {
          pushToast('Reconnected to live updates.', 'success');
        }
      };

      source.addEventListener('overview_changed', handleServerEvent);
      source.addEventListener('resource_changed', handleServerEvent);
      source.addEventListener('schema_changed', handleServerEvent);
      source.onerror = () => {
        source?.close();
        if (!active) {
          return;
        }

        retries += 1;
        if (retries >= 3) {
          pause('Live updates paused.');
          return;
        }

        setLiveUpdates('reconnecting');
        reconnectTimer = window.setTimeout(connect, retries * 1_500);
      };
    }

    connect();
    return () => {
      active = false;
      source?.close();
      if (reconnectTimer !== null) {
        window.clearTimeout(reconnectTimer);
      }
      if (refreshTimer !== null) {
        window.clearTimeout(refreshTimer);
      }
    };
  }, [client, pushToast, refreshQueries, streamKey]);

  return {
    liveUpdates,
    retryLiveUpdates() {
      setStreamKey((current) => current + 1);
    }
  };
}
