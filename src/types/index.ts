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
  backupStatus: 'missing' | 'available' | 'invalid' | 'interrupted' | 'unrecognized' | 'lost'
  git: GitInfo | null
  /** The version line the local checkout declares: a branch alias or a `version`. */
  localVersion: string | null
  /** Whether that version line fits the constraint, when it can be judged at all. */
  constraintStatus: 'satisfied' | 'mismatch' | 'unknown'
  /** Whether the checkout autoloads the way the generated autoloader expects. */
  autoloadStatus: 'matches' | 'differs' | 'unknown'
  /** True when an external `composer install` replaced a link this app recorded. */
  relinkable: boolean
}
export interface Project {
  /** Path below the development root: the name itself, or `<group>/<name>`. */
  id: string
  /** The directory name on its own. */
  name: string
  /** The directories between the root and the project; null when it sits in the root. */
  group: string | null
  composerName: string | null
  path: string
  git: GitInfo | null
  composeFile: string | null
  packages: PackageStatus[]
}
/** One checkout that could back a package which has more than one. */
export interface AmbiguousCandidate {
  projectId: string
  path: string
  /** Two clones of one repository share a name; the branch and commit tell them apart. */
  git: GitInfo | null
  version: string | null
}

/** A package name matching several local checkouts, and the choice made for it. */
export interface AmbiguousPackage {
  packageName: string
  candidates: AmbiguousCandidate[]
  /** The chosen project id, or null while the question is open. */
  chosen: string | null
}

export interface ScanResult {
  developmentRoot: string
  projects: Project[]
  warnings: Message[]
  /** Packages with more than one checkout under this root. The scanner never picks
   *  one by itself, so each of these is a question for the person using the app. */
  ambiguous: AmbiguousPackage[]
}

/** What `composer dump-autoload` did, and where it did it. */
export interface AutoloadDump {
  /** Which PHP regenerated the autoloader. */
  location: 'container' | 'host'
  /** The Compose service it ran in, when it ran in a container. */
  service: string | null
  /** Composer's own output, verbatim and bounded to its last lines. */
  output: string
  /** Why the host was used although the project has containers. */
  message: Message | null
}

/** The tools a project directory can be handed to, as the backend names them. */
export type ExternalTool = 'phpstorm' | 'vscode' | 'terminal' | 'files'

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
  mountPlan?: ContainerMount[]
  canApplyMounts?: boolean
  /** Present only on a synchronisation; absent on a plain inspection. */
  synced?: MountSync | null
}

/** One generated bind mount: a local source made visible at the link's destination. */
export interface ContainerMount {
  packageName: string
  hostPath: string
  containerPath: string
}

/** What a synchronisation changed, so a container restart can be explained. */
export interface MountSync {
  added: ContainerMount[]
  removed: ContainerMount[]
  recreated: boolean
}

/** One preserved Composer version under `vendor/.symfolinker/.history/`. */
export interface ArchiveEntry {
  packageName: string
  /** The archive's directory name, which is also how it is addressed. */
  archiveId: string
  path: string
  version: string | null
  /** Seconds since the Unix epoch; 0 when no timestamp could be recovered. */
  archivedAt: number
  sizeBytes: number
  files: number
  /** True when a sidecar describes this archive and names this package and project. */
  recognized: boolean
  /** False for a file or link that was moved aside: removable, never restorable. */
  directory: boolean
}

/** What a switch needs from the filesystem, checked before one is attempted. */
export interface PreflightReport {
  rootWritable: boolean
  /** False on a filesystem without file locking, such as a \\wsl.localhost root.
   *  Switching still works; a second instance in the same root cannot be noticed. */
  lockingSupported: boolean
  vendorWritable: boolean
  /** What a link here would become, or null when neither kind could be created. */
  linkKind: 'symlink' | 'junction' | null
  freeBytes: number | null
  preservedBytes: number
  /** The first failure's own wording, so a red cross is never just a red cross. */
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
