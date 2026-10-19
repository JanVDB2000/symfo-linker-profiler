<script setup lang="ts">
// Two checkouts of one package under a single root is a deliberate arrangement: one
// clone to test a multi-tenant setup, another on a QA branch so neither has to be
// stashed for the other. The scanner refuses to guess which one a link should use,
// and this is where that question gets answered.
//
// Presented as a list of choices rather than a table: the columns a table would want
// are mostly empty, and what actually distinguishes two clones of one repository is a
// single line of Git state. The choice is stored per development root, so answering
// once serves every project that requires the package. Only an id is ever sent - the
// scan resolves it to a path, so the interface never supplies one.
import { useWorkspace } from '../stores/workspace'
import ProfilerIcon from './ProfilerIcon.vue'
import Spinner from './Spinner.vue'
import { t } from '../i18n'

const workspace = useWorkspace()
</script>

<template>
  <!-- Optional on ambiguous too: a scan restored from an older cache, or any result
       assembled without it, must leave the panel absent rather than take the window
       down over one missing field. -->
  <section v-if="workspace.result?.ambiguous?.length" class="checkout-panel">
    <header class="checkout-heading">
      <strong>{{ t('More than one checkout') }}</strong>
      <p class="checkout-description">{{ t('These packages have several local checkouts under this development root. SymfoLinker does not guess between them, so nothing is linked until one is chosen.') }}</p>
    </header>

    <div v-for="entry in workspace.result.ambiguous" :key="entry.packageName" class="checkout-group">
      <div class="checkout-group-heading">
        <strong class="package-name">{{ entry.packageName }}</strong>
        <button
          v-if="entry.chosen"
          class="text-button checkout-reset"
          :disabled="!!workspace.swapping"
          @click="workspace.setPackageSource(entry.packageName, null)"
        >{{ t('Ask again') }}</button>
      </div>

      <ul class="checkout-list">
        <li v-for="candidate in entry.candidates" :key="candidate.projectId" class="checkout-option" :class="{ chosen: entry.chosen === candidate.projectId }">
          <!-- The id is the path below the root, so it already says where this is;
               the absolute path goes in the title rather than on a second line. -->
          <div class="checkout-identity" :title="candidate.path">
            <span class="checkout-id">{{ candidate.projectId }}</span>
            <span class="checkout-meta">
              <span class="branch"><ProfilerIcon name="branch" />{{ candidate.git?.branch ?? t('No Git information') }}</span>
              <code v-if="candidate.git?.commit" class="commit">{{ candidate.git.commit }}</code>
              <span v-if="candidate.version" class="muted">{{ t('checkout provides {version}', { version: candidate.version }) }}</span>
              <span v-if="candidate.git?.dirty" class="checkout-dirty">{{ t('{count} changed', { count: candidate.git.changedFiles }) }}</span>
            </span>
          </div>
          <span v-if="entry.chosen === candidate.projectId" class="label status-success checkout-action">{{ t('In use') }}</span>
          <button
            v-else
            class="button button-small checkout-action"
            :disabled="!!workspace.swapping"
            @click="workspace.setPackageSource(entry.packageName, candidate.projectId)"
          ><Spinner v-if="workspace.swapping === 'source'" />{{ t('Use this checkout') }}</button>
        </li>
      </ul>
    </div>
  </section>
</template>
