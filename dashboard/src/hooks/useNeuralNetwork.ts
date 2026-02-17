import { useEffect, useRef, useState } from 'react';
import type { Node, Edge, Pulse, ModalState, StrictSystemEvent } from '../types';
import { useTheme } from './useTheme';

const INITIAL_NODES: Node[] = [
  { id: 'karin', label: 'KARIN CORE', x: 0, y: 0, vx: 0, vy: 0, type: 'core', color: '#2e4de6', data: { status: 'OPTIMIZING', lastActive: 'NOW', log: 'Core processing stable...' } },
  { id: 'memory_recall', label: 'MEMORY', x: -200, y: -100, vx: 0, vy: 0, type: 'tool', color: '#2e6be6', data: { status: 'STANDBY', lastActive: 'READY', log: 'Long-term memory interface.' } },
  { id: 'google_search', label: 'GOOGLE', x: 200, y: -100, vx: 0, vy: 0, type: 'tool', color: '#2ea8e6', data: { status: 'STANDBY', lastActive: 'READY', log: 'Web search interface.' } },
  { id: 'discord_research', label: 'DISCORD', x: 300, y: 50, vx: 0, vy: 0, type: 'endpoint', color: '#2ea8e6', data: { status: 'STANDBY', lastActive: 'READY', log: 'Discord log analyzer.' } },
  { id: 'user', label: 'USER', x: -150, y: 150, vx: 0, vy: 0, type: 'endpoint', color: '#2e4de6', data: { status: 'CONNECTED', lastActive: 'NOW', log: 'User interaction gateway.' } },
];

const INITIAL_EDGES: Edge[] = [
  { source: 'karin', target: 'memory_recall', color: '#2e6be6' },
  { source: 'karin', target: 'google_search', color: '#2ea8e6' },
  { source: 'karin', target: 'discord_research', color: '#2ea8e6' },
  { source: 'karin', target: 'user', color: '#2e4de6' },
];

export function useNeuralNetwork(
  mouseRef: React.MutableRefObject<{ x: number, y: number }>,
  events: StrictSystemEvent[],
  onEventProcessed: (timestamp: number) => void,
  seekTime: number | null = null
) {
  const { colors } = useTheme();
  const colorsRef = useRef(colors);
  colorsRef.current = colors;

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const nodes = useRef<Node[]>(INITIAL_NODES);
  const edges = useRef<Edge[]>(INITIAL_EDGES);
  const pulses = useRef<Pulse[]>([]);
  const draggingNode = useRef<Node | null>(null);
  const [selectedModal, setSelectedModal] = useState<ModalState | null>(null);
  const selectedModalRef = useRef<ModalState | null>(null);
  const longPressTimer = useRef<number | null>(null);
  const eventsRef = useRef(events);
  const lastProcessedEventTime = useRef<number>(0);
  
  // Viewport state
  const viewportRef = useRef({ x: 0, y: 0, k: 1 });
  const [viewport, setViewport] = useState({ x: 0, y: 0, k: 1 });
  const isPanning = useRef(false);
  const lastPanPoint = useRef({ x: 0, y: 0 });
  const isVisible = useRef(true);

  useEffect(() => { selectedModalRef.current = selectedModal; }, [selectedModal]);
  useEffect(() => { eventsRef.current = events; }, [events]);

  // Update brand-colored nodes/edges when theme changes
  useEffect(() => {
    nodes.current.forEach(n => {
      if (n.id === 'karin' || n.id === 'user') n.color = colors.brandHex;
    });
    edges.current.forEach(e => {
      if (e.source === 'karin' && e.target === 'user') e.color = colors.brandHex;
    });
  }, [colors.brandHex]);

  const processEvent = (event: StrictSystemEvent, isHistorical = false) => {
    switch (event.type) {
      case "MessageReceived":
        pulses.current.push({
          edge: { source: 'user', target: 'karin', color: colorsRef.current.brandHex },
          progress: 0, speed: 0.04, color: colorsRef.current.brandHex
        });
        break;
      case "RawMessage":
        if (event.payload.message && event.payload.message.role === "user") {
          pulses.current.push({
            edge: { source: 'user', target: 'karin', color: colorsRef.current.brandHex },
            progress: 0, speed: 0.04, color: colorsRef.current.brandHex
          });
        }
        break;
      case "ToolStart": {
        const { node, label, color } = event.payload;
        let target = nodes.current.find(n => n.id === node);
        if (!target) {
          target = { id: node, label: label, x: Math.random()*100, y: Math.random()*100, vx: 0, vy: 0, type: 'tool', color: color, data: { status: 'ACTIVE', lastActive: 'NOW', log: `Initializing ${label}...` } };
          nodes.current.push(target);
          edges.current.push({ source: 'karin', target: node, color: color });
        }
        target.data = { ...target.data!, status: 'ACTIVE', lastActive: 'NOW' };
        pulses.current.push({ edge: { source: 'karin', target: node, color: color }, progress: 0, speed: 0.05, color: color });
        break;
      }
      case "ToolEnd": {
        const node = event.payload.node;
        const target = nodes.current.find(n => n.id === node);
        if (target) {
          target.data = { ...target.data!, status: 'STANDBY', lastActive: 'NOW' };
          pulses.current.push({ edge: { source: node, target: 'karin', color: target.color }, progress: 0, speed: 0.03, color: target.color });
        }
        break;
      }
    }
    if (!isHistorical) onEventProcessed(event.timestamp);
  };

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    let rafId: number;

    const handleResize = () => {
      if (canvas.parentElement) {
        const dpr = Math.min(window.devicePixelRatio || 1, 2);
        const width = canvas.parentElement.clientWidth;
        const height = canvas.parentElement.clientHeight;
        canvas.width = width * dpr;
        canvas.height = height * dpr;
        canvas.style.width = `${width}px`;
        canvas.style.height = `${height}px`;
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        viewportRef.current.x = width / 2;
        viewportRef.current.y = height / 2;
        setViewport({ ...viewportRef.current });
      }
    };

    const resizeObserver = new ResizeObserver(() => handleResize());
    if (canvas.parentElement) resizeObserver.observe(canvas.parentElement);
    handleResize();

    // Pause rendering when canvas is not visible (e.g. behind overlay)
    const intersectionObserver = new IntersectionObserver(
      ([entry]) => { isVisible.current = entry.isIntersecting; },
      { threshold: 0.01 }
    );
    intersectionObserver.observe(canvas);

    const toWorld = (sx: number, sy: number) => {
      const { x, y, k } = viewportRef.current;
      const rect = canvas.getBoundingClientRect();
      return { x: (sx - rect.left - x) / k, y: (sy - rect.top - y) / k };
    };

    const handleWheel = (e: WheelEvent) => {
      e.preventDefault();
      const { x, y, k } = viewportRef.current;
      const zoom = -e.deltaY * 0.001;
      const newK = Math.min(Math.max(0.1, k + zoom), 5);
      const rect = canvas.getBoundingClientRect();
      const mx = e.clientX - rect.left, my = e.clientY - rect.top;
      const wx = (mx - x) / k, wy = (my - y) / k;
      viewportRef.current = { x: mx - wx * newK, y: my - wy * newK, k: newK };
      setViewport({ ...viewportRef.current });
    };

    const handleMouseDown = (e: MouseEvent) => {
      const wp = toWorld(e.clientX, e.clientY);
      const found = nodes.current.find(n => Math.sqrt((n.x - wp.x)**2 + (n.y - wp.y)**2) < 35);
      if (found) {
        draggingNode.current = found;
        setSelectedModal(prev => prev?.nodeId === found.id ? null : { nodeId: found.id, offsetX: 0, offsetY: 0, isDragging: false });
      } else {
        isPanning.current = true;
        lastPanPoint.current = { x: e.clientX, y: e.clientY };
        setSelectedModal(null);
      }
    };

    const handleMouseMove = (e: MouseEvent) => {
      if (draggingNode.current) {
        const wp = toWorld(e.clientX, e.clientY);
        draggingNode.current.x = wp.x; draggingNode.current.y = wp.y;
        draggingNode.current.vx = 0; draggingNode.current.vy = 0;
      } else if (isPanning.current) {
        viewportRef.current.x += e.clientX - lastPanPoint.current.x;
        viewportRef.current.y += e.clientY - lastPanPoint.current.y;
        lastPanPoint.current = { x: e.clientX, y: e.clientY };
        setViewport({ ...viewportRef.current });
      }
    };

    const handleMouseUp = () => { draggingNode.current = null; isPanning.current = false; };

    canvas.addEventListener('mousedown', handleMouseDown);
    canvas.addEventListener('wheel', handleWheel, { passive: false });
    window.addEventListener('mousemove', handleMouseMove);
    window.addEventListener('mouseup', handleMouseUp);

    /**
     * Physics Simulation: Handles node attraction/repulsion and movement.
     */
    const updatePhysics = () => {
      const core = nodes.current.find(n => n.id === 'karin');
      if (!core) return;

      nodes.current.forEach(node => {
        if (node === draggingNode.current || node.id === 'karin') return;
        
        const dx = node.x - core.x;
        const dy = node.y - core.y;
        const distance = Math.sqrt(dx * dx + dy * dy) || 1;
        
        // Orbital attraction
        node.vx -= (dx / distance) * (distance - 220) * 0.005;
        node.vy -= (dy / distance) * (distance - 220) * 0.005;
        
        // Tangential rotation
        node.vx += (dy / distance) * 0.02;
        node.vy -= (dx / distance) * 0.02;
        
        // Damping
        node.vx *= 0.9;
        node.vy *= 0.9;
        
        node.x += node.vx;
        node.y += node.vy;
      });
    };

    /**
     * Rendering Loop: Main draw calls for the canvas.
     */
    const render = () => {
      rafId = requestAnimationFrame(render);
      if (document.hidden || !isVisible.current) return;

      const { x, y, k } = viewportRef.current;
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, canvas.width / dpr, canvas.height / dpr);
      
      const worldMouse = toWorld(mouseRef.current.x, mouseRef.current.y);

      // --- Live Event Processing ---
      if (seekTime === null) {
        const now = Date.now();
        if (lastProcessedEventTime.current === 0) {
           eventsRef.current.filter(e => now - (e.timestamp || 0) < 5000).forEach(e => processEvent(e, true));
           lastProcessedEventTime.current = now;
        } else {
           const newEvents = eventsRef.current.filter(e => (e.timestamp || 0) > lastProcessedEventTime.current);
           newEvents.forEach(e => processEvent(e));
           if (newEvents.length > 0) lastProcessedEventTime.current = Math.max(...newEvents.map(e => e.timestamp));
        }
      }

      updatePhysics();

      ctx.save();
      ctx.translate(x, y);
      ctx.scale(k, k);

      // Draw Edges
      edges.current.forEach(edge => {
        const s = nodes.current.find(n => n.id === edge.source);
        const t = nodes.current.find(n => n.id === edge.target);
        if (s && t) {
          ctx.strokeStyle = `${edge.color}26`; 
          ctx.lineWidth = 2;
          ctx.beginPath(); 
          ctx.moveTo(s.x, s.y); 
          ctx.lineTo(t.x, t.y); 
          ctx.stroke();
        }
      });

      // Draw Pulses
      pulses.current = pulses.current.filter(p => {
        const s = nodes.current.find(n => n.id === p.edge.source);
        const t = nodes.current.find(n => n.id === p.edge.target);
        if (!s || !t) return false;
        p.progress += p.speed; 
        if (p.progress >= 1) return false;
        ctx.fillStyle = p.color; 
        ctx.beginPath(); 
        ctx.arc(s.x + (t.x - s.x) * p.progress, s.y + (t.y - s.y) * p.progress, 2.5, 0, Math.PI * 2); 
        ctx.fill();
        return true;
      });

      // Draw Nodes
      nodes.current.forEach(node => {
        const distSq = (node.x - worldMouse.x) ** 2 + (node.y - worldMouse.y) ** 2;
        const isHovered = distSq < 900; // 30^2
        ctx.fillStyle = colorsRef.current.canvasNodeFill;
        ctx.beginPath();
        ctx.arc(node.x, node.y, isHovered ? 8 : 6, 0, Math.PI * 2);
        ctx.fill();
        ctx.strokeStyle = node.color;
        ctx.lineWidth = 2;
        ctx.stroke();
        ctx.fillStyle = colorsRef.current.canvasText;
        ctx.font = 'bold 10px monospace';
        ctx.textAlign = 'center';
        ctx.fillText(node.label, node.x, node.y + 20);
      });

      ctx.restore();
    };
    render();

    return () => {
      resizeObserver.disconnect();
      intersectionObserver.disconnect();
      canvas.removeEventListener('mousedown', handleMouseDown);
      canvas.removeEventListener('wheel', handleWheel);
      window.removeEventListener('mousemove', handleMouseMove);
      window.removeEventListener('mouseup', handleMouseUp);
      cancelAnimationFrame(rafId);
    };
  }, []);

  return { canvasRef, selectedModal, setSelectedModal, nodes, longPressTimer, viewport };
}