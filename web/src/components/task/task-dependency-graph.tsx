// TaskDependencyGraph — force-directed view of a task and its dependency closure (RFC-043 §D9).
//
// Renders nothing when there are no dependencies (the contract from the
// plan: "TaskDependencyGraph (d3-force, null on empty)"). When deps exist
// it draws the focal task + each dependency as a labeled node connected by
// a directed edge, laid out by d3-force simulation. The simulation runs in
// a rAF loop and the SVG is updated each tick — no React re-renders during
// animation.

import * as d3 from 'd3-force'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { TASK_STATUS_META, type Task } from '@/types/task'

const NODE_W = 140
const NODE_H = 44
const SVG_W = 560
const SVG_H = 320

interface SimNode extends d3.SimulationNodeDatum {
  id: string
  label: string
  status: Task['status']
  /** True for the focal node (rendered distinctly). */
  focal: boolean
}

interface SimLink extends d3.SimulationLinkDatum<SimNode> {
  /** Source/target are string IDs — d3-force resolves them when simulation
   *  starts because every node id is unique. */
  source: string | SimNode
  target: string | SimNode
}

export interface TaskDependencyGraphProps {
  task: Task
  dependencies: Task[]
}

export function TaskDependencyGraph({ task, dependencies }: TaskDependencyGraphProps) {
  const { t } = useTranslation()
  const svgRef = useRef<SVGSVGElement | null>(null)
  // Tick counter to force re-render after the simulation moves nodes.
  const [, setTick] = useState(0)

  const nodes = useMemo<SimNode[]>(() => {
    const focal: SimNode = {
      id: task.id,
      label: task.name,
      status: task.status,
      focal: true,
    }
    const deps: SimNode[] = dependencies.map((d) => ({
      id: d.id,
      label: d.name,
      status: d.status,
      focal: false,
    }))
    return [focal, ...deps]
  }, [task.id, task.name, task.status, dependencies])

  const links = useMemo<SimLink[]>(
    () => dependencies.map((d) => ({ source: task.id, target: d.id })),
    [task.id, dependencies],
  )

  // Run simulation; resolve string source/target to node refs on each tick.
  const resolvedRef = useRef<{ nodes: SimNode[]; links: SimLink[] }>({ nodes, links })

  useEffect(() => {
    if (dependencies.length === 0) return
    const simNodes = nodes.map((n) => ({ ...n }))
    const simLinks: SimLink[] = links.map((l) => ({ source: l.source, target: l.target }))
    const sim = d3
      .forceSimulation<SimNode>(simNodes)
      .force(
        'link',
        d3
          .forceLink<SimNode, SimLink>(simLinks)
          .id((d) => d.id)
          .distance(120)
          .strength(0.7),
      )
      .force('charge', d3.forceManyBody<SimNode>().strength(-260))
      .force('center', d3.forceCenter<SimNode>(SVG_W / 2, SVG_H / 2))
      .force('collide', d3.forceCollide<SimNode>().radius(Math.max(NODE_W, NODE_H) / 1.5))
      .alpha(0.9)
      .alphaDecay(0.04)
      .on('tick', () => {
        resolvedRef.current = { nodes: simNodes, links: simLinks }
        setTick((tk) => (tk + 1) % 1_000_000)
      })

    return () => {
      sim.stop()
    }
  }, [nodes, links, dependencies.length])

  // Contract: render null when there are no dependencies.
  if (dependencies.length === 0) return null

  const renderNodes =
    resolvedRef.current.nodes.length === nodes.length ? resolvedRef.current.nodes : nodes
  const renderLinks =
    resolvedRef.current.links.length === links.length ? resolvedRef.current.links : links

  const getNode = (endpoint: string | SimNode): SimNode => {
    if (typeof endpoint !== 'string') return endpoint
    return renderNodes.find((n) => n.id === endpoint) ?? renderNodes[0]!
  }

  return (
    <div className="space-y-1.5">
      <div className="text-xs font-medium text-muted-foreground">{t('tasks.dependencyGraph')}</div>
      <svg
        ref={svgRef}
        viewBox={`0 0 ${SVG_W} ${SVG_H}`}
        className="w-full h-auto rounded-lg border bg-muted/30"
        role="img"
        aria-label={t('tasks.dependencyGraph')}
      >
        <defs>
          <marker
            id="dep-arrow"
            viewBox="0 -5 10 10"
            refX={NODE_W / 2 + 8}
            refY={0}
            markerWidth={6}
            markerHeight={6}
            orient="auto"
          >
            <path d="M0,-5L10,0L0,5" className="fill-muted-foreground" />
          </marker>
        </defs>

        {/* Edges */}
        {renderLinks.map((link) => {
          const s = getNode(link.source)
          const t2 = getNode(link.target)
          const sx = (s.x ?? 0) + NODE_W / 2
          const sy = (s.y ?? 0) + NODE_H / 2
          const tx = (t2.x ?? 0) + NODE_W / 2
          const ty = (t2.y ?? 0) + NODE_H / 2
          return (
            <line
              key={`${s.id}-${t2.id}`}
              x1={sx}
              y1={sy}
              x2={tx}
              y2={ty}
              className="stroke-muted-foreground"
              strokeWidth={1}
              markerEnd="url(#dep-arrow)"
            />
          )
        })}

        {/* Nodes */}
        {renderNodes.map((n) => {
          const meta = TASK_STATUS_META[n.status]
          return (
            <g
              key={n.id}
              transform={`translate(${(n.x ?? 0).toString()}, ${(n.y ?? 0).toString()})`}
            >
              <rect
                width={NODE_W}
                height={NODE_H}
                rx={8}
                className={n.focal ? 'fill-primary stroke-primary' : 'fill-card stroke-border'}
                strokeWidth={1}
              />
              <circle
                cx={10}
                cy={NODE_H / 2}
                r={4}
                className={n.focal ? 'fill-primary-foreground' : meta.bgColor.replace('/10', '')}
              />
              <text
                x={NODE_W / 2 + 4}
                y={NODE_H / 2 + 4}
                textAnchor="middle"
                className={n.focal ? 'fill-primary-foreground' : 'fill-foreground'}
                fontSize={11}
                fontWeight={500}
              >
                {n.label.length > 16 ? `${n.label.slice(0, 15)}…` : n.label}
              </text>
              <circle
                cx={NODE_W - 8}
                cy={NODE_H / 2}
                r={4}
                className={n.focal ? 'fill-primary-foreground' : meta.bgColor.replace('/10', '')}
              />
            </g>
          )
        })}
      </svg>
    </div>
  )
}
