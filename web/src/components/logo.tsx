"use client";

import { useRef, useCallback, useEffect } from "react";

interface LogoProps {
  size?: number;
  animate?: boolean;
  headOnly?: boolean;
}

export function Logo({ size = 32, animate = false, headOnly = false }: LogoProps) {
  const arrowsRef = useRef<SVGGElement>(null);
  const headRef = useRef<SVGGElement>(null);
  const hoveringRef = useRef(false);

  const stopTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (animate) {
      if (stopTimerRef.current) {
        clearTimeout(stopTimerRef.current);
        stopTimerRef.current = null;
      }
      arrowsRef.current?.classList.remove("paused");
      headRef.current?.classList.remove("paused");
    } else if (!hoveringRef.current) {
      stopTimerRef.current = setTimeout(() => {
        stopTimerRef.current = null;
        if (!hoveringRef.current) {
          arrowsRef.current?.classList.add("paused");
          headRef.current?.classList.add("paused");
        }
      }, 2000);
    }
    return () => {
      if (stopTimerRef.current) {
        clearTimeout(stopTimerRef.current);
        stopTimerRef.current = null;
      }
    };
  }, [animate]);

  const play = useCallback(() => {
    hoveringRef.current = true;
    arrowsRef.current?.classList.remove("paused");
    headRef.current?.classList.remove("paused");
  }, []);

  const stop = useCallback(() => {
    hoveringRef.current = false;
    if (!animate) {
      arrowsRef.current?.classList.add("paused");
      headRef.current?.classList.add("paused");
    }
  }, [animate]);

  return (
    <svg
      width={size}
      height={size}
      viewBox={headOnly ? "22 20 56 56" : "0 0 100 100"}
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      onMouseEnter={play}
      onMouseLeave={stop}
    >
      <style>{`
        .arrows { animation: spin 3s linear infinite; transform-origin: 50px 50px; }
        .arrows.paused { animation-play-state: paused; }
        .head { animation: tilt 3s ease-in-out infinite; transform-origin: 50px 55px; }
        .head.paused { animation-play-state: paused; }
        .arrow-a path { animation: strokeA 3s ease-in-out infinite; }
        .arrow-a polygon { animation: fillA 3s ease-in-out infinite; }
        .arrow-b path { animation: strokeB 3s ease-in-out infinite; }
        .arrow-b polygon { animation: fillB 3s ease-in-out infinite; }
        .arrows.paused .arrow-a path, .arrows.paused .arrow-a polygon,
        .arrows.paused .arrow-b path, .arrows.paused .arrow-b polygon { animation-play-state: paused; }
        ${!animate ? ".arrows, .head, .arrow-a path, .arrow-a polygon, .arrow-b path, .arrow-b polygon { animation-play-state: paused; }" : ""}
        @keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
        @keyframes tilt {
          0%, 100% { transform: rotate(0deg); }
          25% { transform: rotate(8deg); }
          75% { transform: rotate(-8deg); }
        }
        @keyframes strokeA {
          0%, 100% { stroke: #4FD1C5; }
          50% { stroke: #FFFFFF; }
        }
        @keyframes fillA {
          0%, 100% { fill: #4FD1C5; }
          50% { fill: #FFFFFF; }
        }
        @keyframes strokeB {
          0%, 100% { stroke: #FFFFFF; }
          50% { stroke: #4FD1C5; }
        }
        @keyframes fillB {
          0%, 100% { fill: #FFFFFF; }
          50% { fill: #4FD1C5; }
        }
      `}</style>

      {/* Rotating arrows */}
      {!headOnly && <g ref={arrowsRef} className={`arrows${animate ? "" : " paused"}`}>
        <g className="arrow-a">
          {/* Top arrow */}
          <path
            d="M58 14 A38 38 0 0 1 86 42"
            stroke="#4FD1C5"
            strokeWidth="5"
            strokeLinecap="round"
            fill="none"
          />
          <polygon points="87,48 90,34 79,36" fill="#4FD1C5" />
        </g>

        <g className="arrow-b">
          {/* Right arrow */}
          <path
            d="M86 58 A38 38 0 0 1 58 86"
            stroke="#FFFFFF"
            strokeWidth="5"
            strokeLinecap="round"
            fill="none"
          />
          <polygon points="52,87 66,90 64,79" fill="#FFFFFF" />
        </g>

        <g className="arrow-a">
          {/* Bottom arrow */}
          <path
            d="M42 86 A38 38 0 0 1 14 58"
            stroke="#4FD1C5"
            strokeWidth="5"
            strokeLinecap="round"
            fill="none"
          />
          <polygon points="13,52 10,66 21,64" fill="#4FD1C5" />
        </g>

        <g className="arrow-b">
          {/* Left arrow */}
          <path
            d="M14 42 A38 38 0 0 1 42 14"
            stroke="#FFFFFF"
            strokeWidth="5"
            strokeLinecap="round"
            fill="none"
          />
          <polygon points="48,13 34,10 36,21" fill="#FFFFFF" />
        </g>
      </g>}

      <g ref={headRef} className={`head${animate ? "" : " paused"}`}>
        {/* Antenna */}
        <circle cx="50" cy="28" r="4" stroke="#4FD1C5" strokeWidth="3.5" fill="none" />
        <line x1="50" y1="32" x2="50" y2="40" stroke="#4FD1C5" strokeWidth="3.5" strokeLinecap="round" />

        {/* Robot head */}
        <rect
          x="28"
          y="40"
          width="44"
          height="32"
          rx="14"
          stroke="#4FD1C5"
          strokeWidth="5"
          fill="white"
        />

        {/* Left eye > */}
        <polyline
          points="41,52 44,55 41,58"
          stroke="#4FD1C5"
          strokeWidth="3"
          strokeLinecap="round"
          strokeLinejoin="round"
          fill="none"
        />

        {/* Right eye < */}
        <polyline
          points="59,52 56,55 59,58"
          stroke="#4FD1C5"
          strokeWidth="3"
          strokeLinecap="round"
          strokeLinejoin="round"
          fill="none"
        />

        {/* Smile */}
        <path
          d="M42 63 Q50 70 58 63"
          stroke="#4FD1C5"
          strokeWidth="3"
          strokeLinecap="round"
          fill="none"
        />
      </g>
    </svg>
  );
}
