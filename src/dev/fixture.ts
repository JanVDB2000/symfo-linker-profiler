// Fixture workspace for npm run dev: the browser has no Tauri backend.
// Cover all UI modes, link states, backup states and empty data states.
// Include projects without Git information or dependencies.
// Loaded only in browser development; the desktop build does not use fixtures.
import type { Message } from '../i18n/translator'
import type { ArchiveEntry, AutoloadDump, ComposeService, ContainerReport, GitInfo, PackageStatus, PreflightReport, Project, ProjectStatus, ScanResult } from '../types'

const SEP = '\\'
const ROOT = `D:${SEP}dev`
const BACKUP_DIR = `vendor${SEP}.symfolinker-backup`

/** Build Windows paths using a single separator constant; ids carry their group. */
function path(...segments: string[]): string {
  return segments.flatMap(segment => segment.split('/')).join(SEP)
}

/**
 * Where each package's local checkout lives in this workspace.
 *
 * Most of them sit in a group, because most development roots are organised that way:
 * the fixture has to exercise the grouped picker and the cross-group links it draws.
 */
const LOCAL_IDS: Record<string, string> = {
  'acme/payment-sdk': 'library/payment-sdk',
  'acme/shared-kernel': 'library/shared-kernel',
  'acme/http-signing': 'library/http-signing',
  'acme/ui-components': 'library/ui-components',
  'acme/invoice-worker': 'library/invoice-worker',
  'acme/legacy-crm': 'legacy-crm',
}

function git(branch: string, commit: string, changedFiles = 0): GitInfo {
  return { branch, commit, dirty: changedFiles > 0, changedFiles }
}

/** Shared package defaults let each fixture specify only its differences. */
function pkg(host: string, packageName: string, overrides: Partial<PackageStatus> = {}): PackageStatus {
  const [vendor, project] = packageName.split('/')
  const localProjectId = LOCAL_IDS[packageName] ?? project
  return {
    packageName,
    constraint: '^2.4',
    dependencyType: 'require',
    localProjectId,
    localPath: path(ROOT, localProjectId),
    vendorPath: path(ROOT, host, 'vendor', vendor, project),
    backupPath: path(ROOT, host, BACKUP_DIR, vendor, project),
    mode: 'vendor',
    linkStatus: 'notLinked',
    backupStatus: 'missing',
    git: null,
    localVersion: null,
    constraintStatus: 'unknown',
    autoloadStatus: 'unknown',
    relinkable: false,
    ...overrides,
  }
}

const projects: Project[] = [
  {
    id: 'apps/shop-api',
    name: 'shop-api',
    group: 'apps',
    composerName: 'acme/shop-api',
    path: path(ROOT, 'apps/shop-api'),
    git: git('feature/checkout-v2', '9f3c1ab', 7),
    composeFile: 'docker-compose.yml',
    packages: [
      // A healthy link whose checkout is a major version ahead of the constraint:
      // Composer would have refused this, a symlink says nothing.
      pkg('apps/shop-api', 'acme/payment-sdk', {
        constraint: '^3.1', mode: 'local', linkStatus: 'linked', backupStatus: 'available',
        git: git('main', '4b7e220'), localVersion: '4.0', constraintStatus: 'mismatch',
        autoloadStatus: 'matches',
      }),
      // A link without a valid backup appears in Health; its checkout also autoloads
      // differently than the generated autoloader expects.
      pkg('apps/shop-api', 'acme/shared-kernel', {
        constraint: '^1.8', mode: 'local', linkStatus: 'linked', backupStatus: 'invalid',
        git: git('fix/serializer', 'c20d884', 2), autoloadStatus: 'differs',
      }),
      // A link an external composer install replaced: the journal still records one.
      pkg('apps/shop-api', 'acme/invoice-worker', {
        constraint: '^0.9', dependencyType: 'requireDev', mode: 'vendor', linkStatus: 'notLinked',
        backupStatus: 'lost', relinkable: true,
      }),
      pkg('apps/shop-api', 'acme/legacy-crm', {
        constraint: 'dev-main', mode: 'unknown', linkStatus: 'missing', backupStatus: 'missing',
      }),
    ],
  },
  {
    id: 'library/payment-sdk',
    name: 'payment-sdk',
    group: 'library',
    composerName: 'acme/payment-sdk',
    path: path(ROOT, 'library/payment-sdk'),
    git: git('main', '4b7e220'),
    composeFile: null,
    packages: [
      // Create the chain shop-api -> payment-sdk -> shared-kernel.
      pkg('library/payment-sdk', 'acme/shared-kernel', {
        constraint: '^1.8', mode: 'local', linkStatus: 'linked', backupStatus: 'available',
        git: git('fix/serializer', 'c20d884', 2),
      }),
      pkg('library/payment-sdk', 'acme/http-signing', {
        constraint: '^2.0', mode: 'local', linkStatus: 'broken', backupStatus: 'available',
        git: null,
      }),
    ],
  },
  {
    // The end of the chain has no dependencies, producing an empty package table.
    id: 'library/shared-kernel',
    name: 'shared-kernel',
    group: 'library',
    composerName: 'acme/shared-kernel',
    path: path(ROOT, 'library/shared-kernel'),
    git: git('fix/serializer', 'c20d884', 2),
    composeFile: null,
    packages: [],
  },
  {
    id: 'apps/admin-portal',
    name: 'admin-portal',
    group: 'apps',
    composerName: 'acme/admin-portal',
    path: path(ROOT, 'apps/admin-portal'),
    git: git('main', '7e51f0d'),
    composeFile: 'compose.yaml',
    packages: [
      pkg('apps/admin-portal', 'acme/shared-kernel', {
        constraint: '^1.7', mode: 'local', linkStatus: 'unexpectedTarget', backupStatus: 'available',
        git: git('fix/serializer', 'c20d884', 2),
      }),
      pkg('apps/admin-portal', 'acme/ui-components', {
        constraint: '^4.2', dependencyType: 'requireDev', mode: 'local', linkStatus: 'invalid',
        backupStatus: 'missing', git: git('release/4.2', 'a18cc73', 11),
      }),
      // The same install replaced this one too, which is why the offer to restore
      // them spans projects rather than sitting in one.
      pkg('apps/admin-portal', 'acme/payment-sdk', {
        constraint: '^3.0', mode: 'vendor', linkStatus: 'notLinked', backupStatus: 'available',
        git: git('main', '4b7e220'), localVersion: '4.0', constraintStatus: 'mismatch',
        relinkable: true,
      }),
    ],
  },
  {
    // No Composer name and no Git: cover both unavailable states.
    id: 'legacy-crm',
    name: 'legacy-crm',
    group: null,
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
    id: 'library/invoice-worker',
    name: 'invoice-worker',
    group: 'library',
    composerName: 'acme/invoice-worker',
    path: path(ROOT, 'library/invoice-worker'),
    git: git('chore/queue-retry', 'd93aa5e', 1),
    composeFile: 'docker-compose.yml',
    packages: [
      pkg('library/invoice-worker', 'acme/shared-kernel', {
        constraint: '^1.8', mode: 'local', linkStatus: 'linked', backupStatus: 'available',
        git: git('fix/serializer', 'c20d884', 2),
      }),
    ],
  },
  // Add enough projects to exercise sidebar scrolling and filtering.
  ...['api-gateway', 'auth-service', 'content-cms', 'mail-dispatcher', 'media-library',
    'report-builder', 'search-indexer', 'webhook-relay'].map<Project>((name, index) => ({
    id: `services/${name}`,
    name,
    group: 'services',
    composerName: `acme/${name}`,
    path: path(ROOT, 'services', name),
    git: git(index % 3 === 0 ? 'develop' : 'main', `${index}${index}c4e1f`.slice(0, 7), index % 4 === 0 ? index : 0),
    composeFile: index % 2 === 0 ? 'docker-compose.yml' : null,
    packages: [
      pkg(`services/${name}`, 'acme/shared-kernel', {
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
    { key: '{path} is unreadable; package statuses may be incomplete.', params: { path: path('services/media-library', 'vendor') } },
    { key: '{name}: multiple local projects found; automatic mapping skipped.', params: { name: 'acme/billing' } },
  ],
  // The same repository checked out twice: one clone to test a multi-tenant setup,
  // another kept on a QA branch so neither has to be stashed for the other. Two real
  // directories with one Composer name, which is why the branch is what tells them
  // apart rather than the name.
  ambiguous: [
    {
      packageName: 'acme/billing',
      chosen: null,
      candidates: [
        { projectId: 'library/billing', path: path('library/billing'), git: git('main', '4b7e220'), version: '3.2' },
        { projectId: 'library/billing-qa', path: path('library/billing-qa'), git: git('qa/invoice-rounding', '9ac118e', 3), version: '3.2' },
      ],
    },
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
  if (project.id === 'apps/admin-portal') {
    return {
      git,
      docker: {
        engine: 'Docker', available: false, composeFile: project.composeFile, services: [],
        message: { key: 'Docker is unavailable. Is Docker Desktop running?' },
      },
    }
  }
  if (project.id === 'library/invoice-worker') {
    return { git, docker: docker(project, [], { key: 'No containers have been created for this Compose project.' }) }
  }
  // Periodically change a service state to make live updates visible.
  const services = SERVICES.map((name, index) => service(project.name, name, index, (tick + index) % 7 !== 0))
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
        // A switch through the app is never a replaced link, whichever way it went.
        relinkable: false,
      }),
    }),
  }
}

// A fixed day in the past, so the fixture's history reads the same on every run.
const ARCHIVED_AT = 1_760_000_000
const DAY = 86_400

/** Mirrors the preserved versions the backend finds under vendor/.symfolinker. */
export function devArchives(project: Project): ArchiveEntry[] {
  const preserved = project.packages.filter(pkg => pkg.backupStatus === 'available')
  return preserved.flatMap((pkg, index) => {
    const archive = (id: string, moment: number, version: string | null, size: number, recognized: boolean, directory = true) => ({
      packageName: pkg.packageName,
      archiveId: id,
      path: path(project.path, 'vendor', '.symfolinker', '.history', ...pkg.packageName.split('/'), id),
      version,
      archivedAt: moment,
      sizeBytes: size,
      files: directory ? 120 + index * 40 : 0,
      recognized,
      directory,
    })
    return [
      archive(`${(ARCHIVED_AT - index * DAY) * 1000}-4242-0`, ARCHIVED_AT - index * DAY, `3.${index}.1`, 2_400_000 + index * 900_000, true),
      // A leftover from an older version, and something that is not a package at all:
      // both are listed so they can be removed, and neither can be restored.
      ...(index === 0
        ? [
          archive(`${(ARCHIVED_AT - 9 * DAY) * 1000}-1-0`, ARCHIVED_AT - 9 * DAY, null, 1_100_000, false),
          archive('stray-file', 0, null, 0, false, false),
        ]
        : []),
    ]
  })
}

/** Mirrors a successful pre-flight check, so the Health panel can be seen in dev. */
export function devPreflight(): PreflightReport {
  return {
    rootWritable: true,
    lockingSupported: true,
    vendorWritable: true,
    linkKind: 'symlink',
    freeBytes: 42_000_000_000,
    preservedBytes: 6_800_000,
    message: null,
  }
}

/** What regenerating the autoloader looks like when it ran in the PHP container. */
export function devAutoloadDump(): AutoloadDump {
  return {
    location: 'container',
    service: 'php-fpm',
    output: 'Generating optimized autoload files\nGenerated optimized autoload files containing 4821 classes',
    message: null,
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
