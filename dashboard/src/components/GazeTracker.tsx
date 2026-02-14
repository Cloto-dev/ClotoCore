import React from 'react';
import { useGazeDetection } from '../hooks/useGazeDetection';

export const GazeTracker: React.FC = () => {
  const { videoRef, gaze, status, errorMsg, delegateLabel, fpsLabel } = useGazeDetection();

  const confidencePercent = gaze ? Math.round(gaze.confidence * 100) : 0;

  return (
    <div className="fixed top-20 left-1/2 -translate-x-1/2 z-[9999] pointer-events-none flex flex-col items-center gap-2">
      {status === 'loading' && (
        <div className="bg-blue-500/90 text-white text-[10px] px-4 py-2 rounded-full font-bold shadow-xl animate-pulse">
          Loading MediaPipe model...
        </div>
      )}
      {status === 'requesting' && (
        <div className="bg-blue-500/90 text-white text-[10px] px-4 py-2 rounded-full font-bold shadow-xl animate-pulse">
          Requesting camera access...
        </div>
      )}
      {status === 'active' && (
        <div className="bg-green-500/90 text-white text-[10px] px-4 py-2 rounded-full font-bold shadow-xl">
          EYE TRACKING ({delegateLabel} {fpsLabel})
          {gaze && ` — (${gaze.x}, ${gaze.y}) Conf: ${confidencePercent}%`}
          {gaze?.fixated && ' — FIXATED'}
        </div>
      )}
      {status === 'stopped' && (
        <div className="bg-red-500/90 text-white text-[10px] px-4 py-2 rounded-full font-bold shadow-xl">
          EYE TRACKING STOPPED — {errorMsg}
        </div>
      )}
      {(status === 'denied' || status === 'error') && (
        <div className="bg-yellow-500/90 text-white text-[10px] px-4 py-2 rounded-full font-bold shadow-xl">
          EYE TRACKING — {errorMsg}
        </div>
      )}

      <video
        ref={videoRef}
        width={640}
        height={480}
        playsInline
        muted
        className={status === 'active'
          ? 'w-32 h-24 rounded border-2 border-white/50 bg-black opacity-60'
          : 'hidden'
        }
      />
    </div>
  );
};
