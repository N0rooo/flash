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
      format: formatCourant(),
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

// ------------------------------------------- onboarding et réglages
const voile = document.getElementById('voile')
const voileReglages = document.getElementById('voileReglages')
const etapes = [0, 1, 2].map(i => document.getElementById('etape' + i))
const points = [0, 1, 2].map(i => document.getElementById('pt' + i))
const onbRetour = document.getElementById('onbRetour')
const onbSuivant = document.getElementById('onbSuivant')
const onbPasser = document.getElementById('onbPasser')
const destPathOnb = document.getElementById('destPathOnb')
const panneauFormat = document.getElementById('panneauFormat')
const hoteOnb = document.getElementById('hoteFormatOnb')
const hoteReglages = document.getElementById('hoteFormatReglages')
let etape = 0

function montrerEtape(i) {
  etape = i
  etapes.forEach((el, j) => (el.hidden = j !== i))
  points.forEach((el, j) => el.classList.toggle('actif', j === i))
  onbRetour.hidden = i === 0
  onbPasser.hidden = i !== 0
  onbSuivant.textContent = i === etapes.length - 1 ? "C'est parti" : 'Continuer'
  if (i === 2) hoteOnb.appendChild(panneauFormat)
  if (destPathOnb && dest) {
    destPathOnb.textContent = dest
    destPathOnb.classList.remove('empty')
  }
}

function ouvrirOnboarding() {
  voileReglages.hidden = true
  voile.hidden = false
  montrerEtape(0)
}

function fermerOnboarding() {
  voile.hidden = true
  hoteReglages.appendChild(panneauFormat)
  localStorage.setItem('flashOnboarde', 'oui')
}

onbSuivant.addEventListener('click', () => {
  if (etape < etapes.length - 1) montrerEtape(etape + 1)
  else fermerOnboarding()
})
onbRetour.addEventListener('click', () => montrerEtape(Math.max(0, etape - 1)))
onbPasser.addEventListener('click', fermerOnboarding)
document.getElementById('ouvrirReglages').addEventListener('click', () => {
  hoteReglages.appendChild(panneauFormat)
  voileReglages.hidden = false
})
document.getElementById('fermerReglages').addEventListener('click', () => {
  voileReglages.hidden = true
})
document.getElementById('revoirOnb').addEventListener('click', ouvrirOnboarding)
document.getElementById('chooseDestOnb').addEventListener('click', async () => {
  const picked = await invoke('choose_dest')
  if (picked) {
    dest = picked
    localStorage.setItem('dest', dest)
    showDest()
    destPathOnb.textContent = dest
    destPathOnb.classList.remove('empty')
  }
})

if (!localStorage.getItem('flashOnboarde')) ouvrirOnboarding()

// -------------------------------------------------------- format des noms
const LIBELLES = { motcle: 'Mot-clé', jour: 'Jour', mois: 'Mois', annee: 'Année', heure: 'Heure', numero: 'N°' }
const chipsDossiersEl = document.getElementById('chipsDossiers')
const chipsNomEl = document.getElementById('chipsNom')
const motcleEl = document.getElementById('motcle')
const apercuEl = document.getElementById('apercu')

let fmt
try {
  fmt = JSON.parse(localStorage.getItem('flashFormat'))
} catch { /* format illisible : on repart du défaut */ }
if (!fmt || !Array.isArray(fmt.ordre)) fmt = null
if (fmt && !Array.isArray(fmt.dossiers)) {
  // migration depuis la première version (nom seul)
  fmt.dossiers = [
    { id: 'annee', actif: true },
    { id: 'mois', actif: true },
    { id: 'jour', actif: false },
    { id: 'motcle', actif: false },
  ]
}
if (!fmt) {
  fmt = {
    dossiers: [
      { id: 'annee', actif: true },
      { id: 'mois', actif: true },
      { id: 'jour', actif: false },
      { id: 'motcle', actif: false },
    ],
    ordre: [
      { id: 'motcle', actif: false },
      { id: 'jour', actif: true },
      { id: 'mois', actif: true },
      { id: 'annee', actif: false },
      { id: 'heure', actif: false },
      { id: 'numero', actif: true },
    ],
    motcle: '',
  }
}

function sauverFormat() {
  localStorage.setItem('flashFormat', JSON.stringify(fmt))
}

function formatCourant() {
  return {
    elements: fmt.ordre.filter(c => c.actif).map(c => c.id),
    dossiers: fmt.dossiers.filter(c => c.actif).map(c => c.id),
    motcle: fmt.motcle || '',
  }
}

function apercu() {
  const exemple = {
    motcle: (fmt.motcle || 'vacances').trim() || 'vacances',
    jour: '10', mois: 'avril', annee: '2026', heure: '14h32', numero: '2',
  }
  const chemin = fmt.dossiers.filter(c => c.actif).map(c => exemple[c.id])
  let nom = fmt.ordre.filter(c => c.actif).map(c => exemple[c.id])
  if (nom.length === 0) nom = ['10', 'avril']
  apercuEl.textContent = (chemin.length ? chemin.join('/') + '/' : '') + nom.join(' ') + '.jpg'
}

// Glisser maison : dragDropEnabled de Tauri intercepte le drag HTML5,
// on suit donc la souris nous-mêmes.
let glisse = null
let vientDeGlisser = false

function toutDessiner() {
  dessinerBarre(chipsDossiersEl, fmt.dossiers)
  dessinerBarre(chipsNomEl, fmt.ordre)
  motcleEl.hidden = !(
    fmt.dossiers.some(c => c.id === 'motcle' && c.actif) ||
    fmt.ordre.some(c => c.id === 'motcle' && c.actif)
  )
  apercu()
}

function dessinerBarre(conteneur, liste) {
  conteneur.innerHTML = ''
  liste.forEach((c, i) => {
    const el = document.createElement('span')
    el.className = 'chip' + (c.actif ? '' : ' coupe')
    el.textContent = LIBELLES[c.id]
    el.addEventListener('click', () => {
      if (vientDeGlisser) return
      c.actif = !c.actif
      sauverFormat()
      toutDessiner()
    })
    el.addEventListener('mousedown', e => {
      if (e.button !== 0) return
      e.preventDefault()
      glisse = { liste, i, el, conteneur, x: e.clientX, y: e.clientY, bouge: false }
    })
    conteneur.appendChild(el)
  })
}

document.addEventListener('mousemove', e => {
  if (!glisse) return
  if (!glisse.bouge && Math.hypot(e.clientX - glisse.x, e.clientY - glisse.y) > 5) {
    glisse.bouge = true
    glisse.el.classList.add('trainee')
  }
  if (!glisse.bouge) return
  for (const chip of glisse.conteneur.children) chip.classList.remove('survole')
  const sous = document.elementFromPoint(e.clientX, e.clientY)
  const cible = sous && sous.closest('.chip')
  if (cible && cible !== glisse.el && cible.parentElement === glisse.conteneur) {
    cible.classList.add('survole')
  }
})

document.addEventListener('mouseup', e => {
  if (!glisse) return
  const g = glisse
  glisse = null
  if (!g.bouge) return
  vientDeGlisser = true
  setTimeout(() => { vientDeGlisser = false }, 0)
  const sous = document.elementFromPoint(e.clientX, e.clientY)
  const cible = sous && sous.closest('.chip')
  if (cible && cible !== g.el && cible.parentElement === g.conteneur) {
    const j = [...g.conteneur.children].indexOf(cible)
    const [pris] = g.liste.splice(g.i, 1)
    g.liste.splice(j, 0, pris)
    sauverFormat()
  }
  toutDessiner()
})

motcleEl.value = fmt.motcle || ''
motcleEl.addEventListener('input', () => {
  fmt.motcle = motcleEl.value
  sauverFormat()
  apercu()
})

toutDessiner()
