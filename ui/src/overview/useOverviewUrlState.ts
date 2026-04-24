import { startTransition, useEffect, useState } from 'react';

import type { OverviewUrlState } from '../types';
import { buildBrowserQueryString, parseOverviewState } from '../urlState';

export function useOverviewUrlState() {
  const [urlState, setUrlState] = useState<OverviewUrlState>(() => parseOverviewState(window.location.search));

  useEffect(() => {
    function handlePopState() {
      startTransition(() => {
        setUrlState(parseOverviewState(window.location.search));
      });
    }

    window.addEventListener('popstate', handlePopState);
    return () => window.removeEventListener('popstate', handlePopState);
  }, []);

  function commitUrlState(nextState: OverviewUrlState) {
    const queryString = buildBrowserQueryString(nextState);
    const nextUrl = `${window.location.pathname}${queryString}`;
    if (nextUrl !== `${window.location.pathname}${window.location.search}`) {
      window.history.replaceState(null, '', nextUrl);
    }

    startTransition(() => {
      setUrlState(nextState);
    });
  }

  return { urlState, commitUrlState };
}
