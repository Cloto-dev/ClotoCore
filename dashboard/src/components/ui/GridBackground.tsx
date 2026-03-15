/**
 * GridBackground — decorative grid overlay for tech/MemoryCore aesthetic.
 *
 * Pure CSS background, zero state, pointer-events: none.
 *
 * Usage:
 *   As a wrapper:  <GridBackground><div>content</div></GridBackground>
 *   Style-only:    style={gridStyle()} on any element
 */

import type { CSSProperties, ReactNode } from 'react';

export function gridStyle({
  size = 24,
  opacity = 0.04,
  color,
}: {
  size?: number;
  opacity?: number;
  color?: string;
} = {}): CSSProperties {
  const c = color ?? `rgba(255,255,255,${opacity})`;
  return {
    backgroundImage:
      `linear-gradient(to right, ${c} 1px, transparent 1px),` + `linear-gradient(to bottom, ${c} 1px, transparent 1px)`,
    backgroundSize: `${size}px ${size}px`,
  };
}

export function GridBackground({
  size = 24,
  opacity = 0.04,
  color,
  className = '',
  children,
}: {
  size?: number;
  opacity?: number;
  color?: string;
  className?: string;
  children?: ReactNode;
}) {
  const finalColor = color ?? `rgba(255,255,255,${opacity})`;

  return (
    <div
      className={className}
      style={{
        backgroundImage:
          `linear-gradient(to right, ${finalColor} 1px, transparent 1px),` +
          `linear-gradient(to bottom, ${finalColor} 1px, transparent 1px)`,
        backgroundSize: `${size}px ${size}px`,
      }}
    >
      {children}
    </div>
  );
}
