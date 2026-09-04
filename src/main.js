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
// Un style = un exemple concret. Un clic, rien d'autre à comprendre.
const STYLES = [
  { id: 'long', libelle: '10 avril 2026', styles: { jour: '5', mois: 'avril', annee: '2026' }, separateur: ' ' },
  { id: 'abrege', libelle: '10 avr. 26', styles: { jour: '5', mois: 'avr', annee: '26' }, separateur: ' ' },
  { id: 'points', libelle: '10.04.26', styles: { jour: '05', mois: '04', annee: '26' }, separateur: '.' },
  { id: 'tirets', libelle: '10-04-2026', styles: { jour: '05', mois: '04', annee: '2026' }, separateur: '-' },
]
const EXEMPLES = {
  jour: { '5': '10', '05': '10' },
  mois: { avril: 'avril', avr: 'avr.', '4': '4', '04': '04' },
  annee: { '2026': '2026', '26': '26' },
}
const chipsDossiersEl = document.getElementById('chipsDossiers')
const chipsNomEl = document.getElementById('chipsNom')
const chipsStyleEl = document.getElementById('chipsStyle')
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

if (!fmt.style) fmt.style = 'long'

function styleActuel() {
  return STYLES.find(v => v.id === fmt.style) || STYLES[0]
}

function sauverFormat() {
  localStorage.setItem('flashFormat', JSON.stringify(fmt))
}

function formatCourant() {
  const v = styleActuel()
  return {
    elements: fmt.ordre.filter(c => c.actif).map(c => c.id),
    dossiers: fmt.dossiers.filter(c => c.actif).map(c => c.id),
    motcle: fmt.motcle || '',
    styles: v.styles,
    separateur: v.separateur,
  }
}

function apercu() {
  const v = styleActuel()
  const exemple = {
    motcle: (fmt.motcle || 'vacances').trim() || 'vacances',
    jour: EXEMPLES.jour[v.styles.jour],
    mois: EXEMPLES.mois[v.styles.mois],
    annee: EXEMPLES.annee[v.styles.annee],
    heure: '14h32',
    numero: '2',
  }
  const chemin = fmt.dossiers
    .filter(c => c.actif)
    .map(c => (exemple[c.id] || '').replace(/\.$/, ''))
  let nom = fmt.ordre.filter(c => c.actif).map(c => exemple[c.id])
  if (nom.length === 0) nom = [exemple.jour, exemple.mois]
  const nomJoint = nom.join(v.separateur).replace(/[. ]+$/, '')
  apercuEl.textContent = (chemin.length ? chemin.join('/') + '/' : '') + nomJoint + '.jpg'
}

// Glisser maison : dragDropEnabled de Tauri intercepte le drag HTML5, on
// suit donc la souris nous-mêmes — bulle fantôme sous le curseur, pastille
// d'origine en creux, réordonnancement en direct pendant le geste.
let glisse = null
let vientDeGlisser = false

function dessinerStyles() {
  chipsStyleEl.innerHTML = ''
  for (const v of STYLES) {
    const el = document.createElement('span')
    el.className = 'chip' + (fmt.style === v.id ? '' : ' coupe')
    el.textContent = v.libelle
    el.addEventListener('click', () => {
      fmt.style = v.id
      sauverFormat()
      toutDessiner()
    })
    chipsStyleEl.appendChild(el)
  }
}

function toutDessiner() {
  dessinerBarre(chipsDossiersEl, fmt.dossiers)
  dessinerBarre(chipsNomEl, fmt.ordre)
  dessinerStyles()
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
    el.dataset.id = c.id
    el.addEventListener('click', () => {
      if (vientDeGlisser) return
      c.actif = !c.actif
      sauverFormat()
      toutDessiner()
    })
    el.addEventListener('mousedown', e => {
      if (e.button !== 0) return
      e.preventDefault()
      const r = el.getBoundingClientRect()
      glisse = {
        liste, el, conteneur,
        x: e.clientX, y: e.clientY,
        dx: e.clientX - r.left, dy: e.clientY - r.top,
        bouge: false, fantome: null,
      }
    })
    conteneur.appendChild(el)
  })
}

document.addEventListener('mousemove', e => {
  if (!glisse) return
  if (!glisse.bouge) {
    if (Math.hypot(e.clientX - glisse.x, e.clientY - glisse.y) <= 5) return
    glisse.bouge = true
    const fantome = glisse.el.cloneNode(true)
    fantome.classList.add('fantome')
    fantome.style.width = glisse.el.getBoundingClientRect().width + 'px'
    document.body.appendChild(fantome)
    glisse.fantome = fantome
    glisse.el.classList.add('creux')
    document.body.classList.add('attrape')
  }
  glisse.fantome.style.left = e.clientX - glisse.dx + 'px'
  glisse.fantome.style.top = e.clientY - glisse.dy + 'px'
  // La bulle est en pointer-events:none : elementFromPoint voit à travers.
  const sous = document.elementFromPoint(e.clientX, e.clientY)
  const cible = sous && sous.closest('.chip')
  if (cible && cible !== glisse.el && cible.parentElement === glisse.conteneur) {
    const r = cible.getBoundingClientRect()
    const avant = e.clientX < r.left + r.width / 2
    glisse.conteneur.insertBefore(glisse.el, avant ? cible : cible.nextSibling)
  }
})

document.addEventListener('mouseup', () => {
  if (!glisse) return
  const g = glisse
  glisse = null
  if (!g.bouge) return
  vientDeGlisser = true
  setTimeout(() => { vientDeGlisser = false }, 0)
  g.fantome.remove()
  g.el.classList.remove('creux')
  document.body.classList.remove('attrape')
  // L'ordre final est celui du DOM, réordonné en direct pendant le geste.
  const ordreIds = [...g.conteneur.children].map(c => c.dataset.id)
  g.liste.sort((a, b) => ordreIds.indexOf(a.id) - ordreIds.indexOf(b.id))
  sauverFormat()
  toutDessiner()
})

motcleEl.value = fmt.motcle || ''
motcleEl.addEventListener('input', () => {
  fmt.motcle = motcleEl.value
  sauverFormat()
  apercu()
})

toutDessiner()
