// Fixture workspace for npm run dev: the browser has no Tauri backend.
// Cover all UI modes, link states, backup states and empty data states.
// Include projects without Git information or dependencies.
// Loaded only in browser development; the desktop build does not use fixtures.
import type { Message } from '../i18n/translator'
import type { ComposeService, ContainerReport, GitInfo, PackageStatus, Project, ProjectStatus, ScanResult } from '../types'

const SEP = '\\'
const ROOT = `D:${SEP}dev`
const BACKUP_DIR = `vendor${SEP}.symfolinker-backup`

/** Build Windows paths using a single separator constant. */
function path(...segments: string[]): string { return segments.join(SEP) }

function git(branch: string, commit: string, changedFiles = 0): GitInfo {
  return { branch, commit, dirty: changedFiles > 0, changedFiles }
}

/** Shared package defaults let each fixture specify only its differences. */
function pkg(host: string, packageName: string, overrides: Partial<PackageStatus> = {}): PackageStatus {
  const [vendor, project] = packageName.split('/')
  return {
    packageName,
    constraint: '^2.4',
    dependencyType: 'require',
    localProjectId: project,
    localPath: path(ROOT, project),
    vendorPath: path(ROOT, host, 'vendor', vendor, project),
    backupPath: path(ROOT, host, BACKUP_DIR, vendor, project),
    mode: 'vendor',
    linkStatus: 'notLinked',
    backupStatus: 'missing',
    git: null,
    ...overrides,
  }
}

const projects: Project[] = [
  {
    id: 'shop-api',
    name: 'shop-api',
    composerName: 'acme/shop-api',
    path: path(ROOT, 'shop-api'),
    git: git('feature/checkout-v2', '9f3c1ab', 7),
    composeFile: 'docker-compose.yml',
    packages: [
      // A healthy link with a backup: the happy path.
      pkg('shop-api', 'acme/payment-sdk', {
        constraint: '^3.1', mode: 'local', linkStatus: 'linked', backupStatus: 'available',
        git: git('main', '4b7e220'),
      }),
      // A link without a valid backup appears in Health.
      pkg('shop-api', 'acme/shared-kernel', {
        constraint: '^1.8', mode: 'local', linkStatus: 'linked', backupStatus: 'invalid',
        git: git('fix/serializer', 'c20d884', 2),
      }),
      pkg('shop-api', 'acme/invoice-worker', {
        constraint: '^0.9', dependencyType: 'requireDev', mode: 'vendor', linkStatus: 'notLinked',
        backupStatus: 'missing',
      }),
      pkg('shop-api', 'acme/legacy-crm', {
        constraint: 'dev-main', mode: 'unknown', linkStatus: 'missing', backupStatus: 'missing',
      }),
    ],
  },
  {
    id: 'payment-sdk',
    name: 'payment-sdk',
    composerName: 'acme/payment-sdk',
    path: path(ROOT, 'payment-sdk'),
    git: git('main', '4b7e220'),
    composeFile: null,
    packages: [
      // Create the chain shop-api -> payment-sdk -> shared-kernel.
      pkg('payment-sdk', 'acme/shared-kernel', {
        constraint: '^1.8', mode: 'local', linkStatus: 'linked', backupStatus: 'available',
        git: git('fix/serializer', 'c20d884', 2),
      }),
      pkg('payment-sdk', 'acme/http-signing', {
        constraint: '^2.0', mode: 'local', linkStatus: 'broken', backupStatus: 'available',
        git: null,
      }),
    ],
  },
  {
    // The end of the chain has no dependencies, producing an empty package table.
    id: 'shared-kernel',
    name: 'shared-kernel',
    composerName: 'acme/shared-kernel',
    path: path(ROOT, 'shared-kernel'),
    git: git('fix/serializer', 'c20d884', 2),
    composeFile: null,
    packages: [],
  },
  {
    id: 'admin-portal',
    name: 'admin-portal',
    composerName: 'acme/admin-portal',
    path: path(ROOT, 'admin-portal'),
    git: git('main', '7e51f0d'),
    composeFile: 'compose.yaml',
    packages: [
      pkg('admin-portal', 'acme/shared-kernel', {
        constraint: '^1.7', mode: 'local', linkStatus: 'unexpectedTarget', backupStatus: 'available',
        git: git('fix/serializer', 'c20d884', 2),
      }),
      pkg('admin-portal', 'acme/ui-components', {
        constraint: '^4.2', dependencyType: 'requireDev', mode: 'local', linkStatus: 'invalid',
        backupStatus: 'missing', git: git('release/4.2', 'a18cc73', 11),
      }),
      pkg('admin-portal', 'acme/payment-sdk', {
        constraint: '^3.0', mode: 'vendor', linkStatus: 'notLinked', backupStatus: 'available',
        git: git('main', '4b7e220'),
      }),
    ],
  },
  {
    // No Composer name and no Git: cover both unavailable states.
    id: 'legacy-crm',
    name: 'legacy-crm',
    composerName: null,
    path: path(ROOT, 'legacy-crm'),
    git: null,
    composeFile: null,
    packages: [
      pkg('legacy-crm', 'acme/shared-kernel', {
        constraint: '0.4.*', mode: 'unknown', linkStatus: 'invalid', backupStatus: 'invalid',
      }),
    ],
  },
  {
    id: 'invoice-worker',
    name: 'invoice-worker',
    composerName: 'acme/invoice-worker',
    path: path(ROOT, 'invoice-worker'),
    git: git('chore/queue-retry', 'd93aa5e', 1),
    composeFile: 'docker-compose.yml',
    packages: [
      pkg('invoice-worker', 'acme/shared-kernel', {
        constraint: '^1.8', mode: 'local', linkStatus: 'linked', backupStatus: 'available',
        git: git('fix/serializer', 'c20d884', 2),
      }),
    ],
  },
  // Add enough projects to exercise sidebar scrolling and filtering.
  ...['api-gateway', 'auth-service', 'content-cms', 'mail-dispatcher', 'media-library',
    'report-builder', 'search-indexer', 'webhook-relay'].map<Project>((name, index) => ({
    id: name,
    name,
    composerName: `acme/${name}`,
    path: path(ROOT, name),
    git: git(index % 3 === 0 ? 'develop' : 'main', `${index}${index}c4e1f`.slice(0, 7), index % 4 === 0 ? index : 0),
    composeFile: index % 2 === 0 ? 'docker-compose.yml' : null,
    packages: [
      pkg(name, 'acme/shared-kernel', {
        constraint: '^1.8',
        mode: index % 2 === 0 ? 'local' : 'vendor',
        linkStatus: index % 2 === 0 ? 'linked' : 'notLinked',
        backupStatus: index % 2 === 0 ? 'available' : 'missing',
        git: git('fix/serializer', 'c20d884', 2),
      }),
    ],
  })),
]

export const devFixture: ScanResult = {
  developmentRoot: ROOT,
  projects,
  warnings: [
    { key: '{path} has no "name" field; the project was identified by its directory name.', params: { path: path('legacy-crm', 'composer.json') } },
    { key: '{path} is unreadable; package statuses may be incomplete.', params: { path: path('media-library', 'vendor') } },
  ],
}

// Vary fixture data on every poll to demonstrate live updates.
// This makes refreshes visible during development.
let tick = 0
const SERVICES = ['php-fpm', 'nginx', 'postgres', 'redis']
const HEALTH: Record<string, string> = { postgres: 'healthy', 'php-fpm': 'starting' }

function service(project: string, name: string, index: number, up: boolean): ComposeService {
  return {
    name: `${project}-${name}-1`,
    service: name,
    state: up ? 'running' : 'exited',
    status: up ? `Up ${2 + index} minutes` : 'Exited (0) 5 minutes ago',
    health: up ? HEALTH[name] ?? null : null,
    ports: up ? `${8000 + index * 10}->${80 + index}/tcp` : '',
    running: up,
  }
}

function docker(project: Project, services: ComposeService[], message: Message | null) {
  return { engine: 'Docker', available: true, composeFile: project.composeFile, services, message }
}

export function devProjectStatus(project: Project): ProjectStatus {
  tick += 1
  // Vary Git state around the scanned value, including occasional clean working trees.
  const changedFiles = project.git ? Math.max(0, project.git.changedFiles + (tick % 5) - 2) : 0
  const git: GitInfo | null = project.git
    ? { ...project.git, changedFiles, dirty: changedFiles > 0 }
    : null

  if (!project.composeFile) {
    return { git, docker: docker(project, [], { key: 'This project has no Compose file; it runs natively.' }) }
  }
  // Fixed fixtures for the two exceptional runtime states.
  if (project.id === 'admin-portal') {
    return {
      git,
      docker: {
        engine: 'Docker', available: false, composeFile: project.composeFile, services: [],
        message: { key: 'Docker is unavailable. Is Docker Desktop running?' },
      },
    }
  }
  if (project.id === 'invoice-worker') {
    return { git, docker: docker(project, [], { key: 'No containers have been created for this Compose project.' }) }
  }
  // Periodically change a service state to make live updates visible.
  const services = SERVICES.map((name, index) => service(project.id, name, index, (tick + index) % 7 !== 0))
  return { git, docker: docker(project, services, null) }
}

/** Mirrors what the backend reports after a switch, so dev can exercise the toggle. */
export function devSwap(
  current: ScanResult,
  projectId: string,
  packageName: string,
  target: 'local' | 'vendor',
): ScanResult {
  const local = target === 'local'
  return {
    ...current,
    projects: current.projects.map(project => project.id !== projectId ? project : {
      ...project,
      packages: project.packages.map(pkg => pkg.packageName !== packageName ? pkg : {
        ...pkg,
        mode: local ? 'local' : 'vendor',
        linkStatus: local ? 'linked' : 'notLinked',
        backupStatus: local ? 'available' : 'missing',
      }),
    }),
  }
}

const CONTAINER_ROOT = '/var/www'

/** Mirrors what the backend reports for container inspection. */
export function devContainerReport(project: Project): ContainerReport {
  if (!project.composeFile) {
    return { services: [], phpService: null, suggested: false, volumes: [], projectContainerPath: null, checks: [],
      message: { key: 'This project has no Compose file; it runs natively.' } }
  }
  const toContainer = (hostPath: string) => hostPath.replace(ROOT, CONTAINER_ROOT).split(SEP).join('/')
  return {
    services: SERVICES,
    phpService: 'php-fpm',
    suggested: true,
    volumes: [[ROOT, CONTAINER_ROOT]],
    projectContainerPath: toContainer(project.path),
    checks: project.packages.filter(pkg => pkg.mode === 'local').map((pkg, index) => ({
      packageName: pkg.packageName,
      containerPath: toContainer(pkg.vendorPath),
      // Laat een mismatch zien zodat de foutweergave testbaar is.
      exists: index % 3 !== 2,
      linkTarget: index % 3 !== 2 ? toContainer(pkg.localPath) : null,
      mapped: true,
    })),
    message: null,
  }
}
