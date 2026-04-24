import { useState } from 'react';

import type { ToastMessage } from './shared';

export function useOverviewToasts() {
  const [toasts, setToasts] = useState<ToastMessage[]>([]);

  function pushToast(message: string, tone: ToastMessage['tone']) {
    const id = Date.now() + Math.floor(Math.random() * 1000);
    setToasts((current) => [...current, { id, tone, message }]);
    window.setTimeout(() => {
      setToasts((current) => current.filter((toast) => toast.id !== id));
    }, 3_000);
  }

  return { toasts, pushToast };
}
