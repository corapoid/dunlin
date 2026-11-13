/**
 * Adapted from the free MIT ReUI Frame, base-nova registry, 2026-11-10.
 * Source: https://reui.io/r/base-nova/frame.json
 * Copyright (c) 2025 Keenthemes Inc. See ../../../REUI-LICENSE.txt.
 * Kept the default composition; omitted unused CVA variants and its dependency.
 */
import type { ComponentProps } from 'react';

export function Frame({ className = '', ...props }: ComponentProps<'div'>) {
  return <div data-slot="frame" className={[
    'relative flex flex-col border bg-muted/50 bg-clip-padding rounded-(--frame-radius)',
    'gap-(--frame-gap) px-(--frame-px) py-(--frame-py)',
    '[--frame-radius:var(--radius-xl)] [--frame-gap:--spacing(0.75)]',
    '[--frame-px:--spacing(0.75)] [--frame-py:--spacing(0.75)]',
    '[--frame-panel-bg:var(--color-card)] [--frame-panel-border-color:var(--color-border)]',
    '[--frame-panel-radius:calc(var(--frame-radius)-var(--frame-px)-1px)]',
    '[--frame-panel-px:--spacing(4)] [--frame-panel-py:--spacing(4)]',
    '[--frame-panel-header-px:--spacing(4)] [--frame-panel-header-py:--spacing(2)]',
    className,
  ].join(' ')} {...props} />;
}

export function FramePanel({ className = '', fit, ...props }: ComponentProps<'div'> & { fit?: boolean }) {
  return <div data-slot="frame-panel" className={[
    'relative overflow-hidden rounded-(--frame-panel-radius) border border-(--frame-panel-border-color)',
    'bg-(--frame-panel-bg) bg-clip-padding shadow-xs',
    'px-(--frame-panel-px) py-(--frame-panel-py)',
    fit ? '' : 'grow', className,
  ].join(' ')} {...props} />;
}

export function FrameHeader({ className = '', ...props }: ComponentProps<'header'>) {
  return <header data-slot="frame-panel-header" className={`flex flex-col px-(--frame-panel-header-px) py-(--frame-panel-header-py) ${className}`} {...props} />;
}

export function FrameTitle({ className = '', ...props }: ComponentProps<'div'>) {
  return <div data-slot="frame-panel-title" className={`text-sm font-semibold ${className}`} {...props} />;
}

export function FrameDescription({ className = '', ...props }: ComponentProps<'div'>) {
  return <div data-slot="frame-panel-description" className={`text-sm text-muted-foreground ${className}`} {...props} />;
}
