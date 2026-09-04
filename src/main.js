const { invoke } = window.__TAURI__.core
const { listen } = window.__TAURI__.event
const { getCurrentWebview } = window.__TAURI__.webview

const destPathEl = document.getElementById('destPath')
const chooseDestBtn = document.getElementById('chooseDest')
const moveToggle = document.getElementById('moveToggle')
const dropzone = document.getElementById('dropzone')
const progressEl = document.getElementById('progress')
const barFill = document.getElementById('barFill')
const progressText = document.getElementById('progressText')
const resultEl = document.getElementById('result')
const resultText = document.getElementById('resultText')
const openDestBtn = document.getElementById('openDest')
const uncertainList = document.getElementById('uncertainList')
const errorList = document.getElementById('errorList')

let dest = localStorage.getItem('dest') || null
let running = false

function showDest() {
  if (dest) {
    destPathEl.textContent = dest
    destPathEl.classList.remove('empty')
  } else {
    destPathEl.textContent = 'Aucun dossier choisi'
    destPathEl.classList.add('empty')
  }
}

moveToggle.checked = localStorage.getItem('move') === '1'
moveToggle.addEventListener('change', () => {
  localStorage.setItem('move', moveToggle.checked ? '1' : '0')
})

chooseDestBtn.addEventListener('click', async () => {
  const picked = await invoke('choose_dest')
  if (picked) {
    dest = picked
    localStorage.setItem('dest', dest)
    showDest()
  }
})

openDestBtn.addEventListener('click', () => {
  if (dest) invoke('open_dest', { path: dest })
})

function setProgress(p) {
  if (p.phase === 'scan') {
    barFill.style.width = '0%'
    progressText.textContent = p.message || 'Flash renifle les dossiers…'
  } else if (p.phase === 'analyze') {
    const pct = p.total ? Math.round((p.done / p.total) * 50) : 0
    barFill.style.width = pct + '%'
    progressText.textContent = `Flash lit les dates de prise de vue… ${p.done}/${p.total}`
  } else if (p.phase === 'apply') {
    const pct = p.total ? 50 + Math.round((p.done / p.total) * 50) : 100
    barFill.style.width = pct + '%'
    progressText.textContent = `Flash rapporte… ${p.done}/${p.total}`
  }
}

listen('progress', e => setProgress(e.payload))

async function runImport(paths) {
  if (running || paths.length === 0) return
  if (!dest) {
    const picked = await invoke('choose_dest')
    if (!picked) return
    dest = picked
    localStorage.setItem('dest', dest)
    showDest()
  }
  running = true
  dropzone.classList.add('disabled')
  resultEl.hidden = true
  errorList.innerHTML = ''
  uncertainList.innerHTML = ''
  progressEl.hidden = false
  barFill.style.width = '0%'
  progressText.textContent = "Flash s'échauffe…"

  try {
    const s = await invoke('import', {
      sources: paths,
      dest,
      moveFiles: moveToggle.checked,
    })
    const parts = [`${s.imported} fichier${s.imported > 1 ? 's' : ''} rapporté${s.imported > 1 ? 's' : ''}`]
    if (s.duplicates) parts.push(`${s.duplicates} doublon${s.duplicates > 1 ? 's' : ''} ignoré${s.duplicates > 1 ? 's' : ''}`)
    if (s.renamed) parts.push(`${s.renamed} renuméroté${s.renamed > 1 ? 's' : ''}`)
    if (s.updated) parts.push(`${s.updated} date${s.updated > 1 ? 's' : ''} corrigée${s.updated > 1 ? 's' : ''}`)
    if (s.errors.length) parts.push(`${s.errors.length} erreur${s.errors.length > 1 ? 's' : ''}`)
    resultText.textContent = parts.join(' · ')
    if (s.uncertain.length) {
      const head = document.createElement('li')
      head.className = 'head'
      head.textContent = `${s.uncertain.length} fichier${s.uncertain.length > 1 ? 's' : ''} sans date fiable, déposé${s.uncertain.length > 1 ? 's' : ''} dans « à vérifier » :`
      uncertainList.appendChild(head)
      for (const name of s.uncertain.slice(0, 30)) {
        const li = document.createElement('li')
        li.textContent = name
        uncertainList.appendChild(li)
      }
      if (s.uncertain.length > 30) {
        const li = document.createElement('li')
        li.textContent = `… et ${s.uncertain.length - 30} autres`
        uncertainList.appendChild(li)
      }
    }
    for (const err of s.errors.slice(0, 50)) {
      const li = document.createElement('li')
      li.textContent = err
      errorList.appendChild(li)
    }
  } catch (err) {
    resultText.textContent = `Erreur : ${err}`
  } finally {
    running = false
    dropzone.classList.remove('disabled')
    progressEl.hidden = true
    resultEl.hidden = false
  }
}

getCurrentWebview().onDragDropEvent(event => {
  const t = event.payload.type
  if (t === 'enter' || t === 'over') {
    if (!running) dropzone.classList.add('hover')
  } else if (t === 'leave') {
    dropzone.classList.remove('hover')
  } else if (t === 'drop') {
    dropzone.classList.remove('hover')
    runImport(event.payload.paths || [])
  }
})

showDest()
