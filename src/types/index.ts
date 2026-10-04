import type { Message } from '../i18n/translator'

export type PackageMode = 'vendor' | 'local' | 'unknown'
export interface GitInfo { branch: string; commit: string; dirty: boolean; changedFiles: number }
export interface PackageStatus {
  packageName: string
  constraint: string
  dependencyType: 'require' | 'requireDev'
  localProjectId: string
  localPath: string
  vendorPath: string
  backupPath: string
  mode: PackageMode
  linkStatus: 'notLinked' | 'linked' | 'broken' | 'unexpectedTarget' | 'missing' | 'invalid'
  backupStatus: 'missing' | 'available' | 'invalid'
  git: GitInfo | null
}
export interface Project {
  id: string
  name: string
  composerName: string | null
  path: string
  git: GitInfo | null
  composeFile: string | null
  packages: PackageStatus[]
}
export interface ScanResult { developmentRoot: string; projects: Project[]; warnings: Message[] }

export interface ComposeService {
  name: string
  service: string
  state: string
  status: string
  health: string | null
  ports: string
  running: boolean
}
export interface DockerStatus {
  /** Which engine answered: Docker or Podman. */
  engine: string | null
  available: boolean
  composeFile: string | null
  services: ComposeService[]
  message: Message | null
}
/** Live project status: refreshed Git state and container status. */
export interface ProjectStatus { git: GitInfo | null; docker: DockerStatus }

export interface ContainerCheck {
  packageName: string
  containerPath: string
  exists: boolean
  linkTarget: string | null
  /** False when no bind mount covers this path, so nothing could be checked. */
  mapped: boolean
}
/** Mounts and per-package validation inside the container. */
export interface ContainerReport {
  services: string[]
  phpService: string | null
  /** True when the service was guessed rather than chosen. */
  suggested: boolean
  volumes: [string, string][]
  projectContainerPath: string | null
  checks: ContainerCheck[]
  message: Message | null
}

/** One local package and every project that currently links to it. */
export interface LinkedPackage {
  packageName: string
  localProjectId: string
  localPath: string
  /** Git state of the local source, which is the branch a consumer actually gets. */
  git: GitInfo | null
  usedBy: { id: string; name: string }[]
}
