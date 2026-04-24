import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { OverviewAppRoot } from './OverviewApp';
import './tailwind.generated.css';
import '@xyflow/react/dist/style.css';

const mountNode = document.getElementById('overview-root');

if (mountNode) {
  const overviewEndpoint = mountNode.getAttribute('data-overview-endpoint') ?? '/overview.json';
  createRoot(mountNode).render(
    <StrictMode>
      <OverviewAppRoot overviewEndpoint={overviewEndpoint} />
    </StrictMode>
  );
}
