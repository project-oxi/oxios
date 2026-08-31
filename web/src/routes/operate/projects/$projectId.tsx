import { createFileRoute } from '@tanstack/react-router'
import { ProjectControlRoom } from '@/components/operate/project-control-room'

export const Route = createFileRoute('/operate/projects/$projectId')({
  component: ProjectControlRoomRoute,
})

function ProjectControlRoomRoute() {
  const { projectId } = Route.useParams()
  return <ProjectControlRoom projectId={projectId} />
}
