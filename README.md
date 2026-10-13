# OpenSheet

Un tableur open source qui reprend l'interface et le comportement d'Excel, écrit en Rust avec [GPUI](https://gpui.rs/).

![OpenSheet](docs/screenshot.jpg)

> État : **v0.1, en développement actif**. La plupart des grandes fonctions d'Excel sont en place, scripts Office compris (les macros VBA sont conservées et lisibles, pas exécutées) ; plusieurs restent partielles ou absentes — voir [Ce qui manque](#ce-qui-manque).

## Lancer

```bash
cargo run -p opensheet --release
```

Avec des données d'exemple :

```bash
cargo run -p opensheet --release -- --demo
```

Ouvrir un fichier :

```bash
cargo run -p opensheet --release -- chemin/vers/classeur.xlsx
```

Convertir sans ouvrir de fenêtre (le format suit l'extension : `.xlsx`, `.csv`, `.tsv`) :

```bash
cargo run -p opensheet --release -- classeur.xlsx --export classeur.csv
```

Créer une vraie application macOS (`target/release/OpenSheet.app`) :

```bash
cargo build -p opensheet --release && scripts/bundle.sh release
```

Prérequis : Rust stable récent et Xcode (GPUI compile ses shaders Metal). Développé et testé sur macOS.

### Langues

Cinq langues : français, anglais, espagnol, allemand, italien. L'interface **et** les formules suivent la langue choisie : `=SOMME(A1:A3;1,5)` en français, `=SUM(A1:A3,1.5)` en anglais, `=SUMME(A1:A3;1,5)` en allemand, `=SUMA(…)` en espagnol, `=SOMMA(…)` en italien — avec les séparateurs, les valeurs `VRAI` / `WAHR` / `VERDADERO`, les erreurs (`#VALEUR!`, `#WERT!`, `#¡VALOR!`) et les formats de date de chaque langue.

- Au démarrage, la langue du système est utilisée.
- Pour en changer : menu **Fichier**, en bas. Le choix est mémorisé.
- Pour forcer une langue au lancement : `OPENSHEET_LANG=de` (ou `fr`, `en`, `es`, `it`).

Les formules sont enregistrées sous forme canonique : un classeur saisi en français s'affiche en allemand sans rien perdre.

Toute l'interface est traduite dans les cinq langues (3 093 textes par langue), avec les termes d'Excel dans chacune. `python3 scripts/check_i18n.py` vérifie qu'aucun texte n'est oublié.

### Polices

Excel utilise Aptos Narrow par défaut, qui n'est pas une police système. Sans elle, OpenSheet affiche une police système de proportions proches (Seravek sur macOS). Deux façons d'obtenir le rendu exact :

- installer Aptos (téléchargement gratuit chez Microsoft) : elle est utilisée automatiquement ;
- ou, si Microsoft Office est installé sur la machine, lancer avec `OPENSHEET_OFFICE_FONTS=1` : les polices Aptos / Calibri de votre installation Office sont chargées à l'exécution (rien n'est copié ni redistribué — vérifiez que votre licence Office vous y autorise).

## Ce qui fonctionne

**Interface**
- Barre de titre, ruban à onglets (Accueil, Insertion, Mise en page, Formules, Données, Révision, Affichage…) avec ses onglets contextuels (tableau, tableau croisé, graphique, graphique croisé, segment, chronologie, forme, image), barre de formule avec zone Nom, onglets de feuilles, barre d'état avec Moyenne / Nb / Somme et zoom.
- Boîtes de dialogue, volets latéraux et menus contextuels d'Excel, dont Format de cellule et ses six onglets.
- Grille de 1 048 576 lignes × 16 384 colonnes, défilement fluide, zoom 10–400 %, volets figés.
- Préférences (⌘,) : général, affichage, modification, calcul, vérification des erreurs, correction automatique, listes personnalisées, tableaux et filtres.
- Correction automatique pendant la saisie (liste modifiable, 2e majuscule, début de phrase, jours) ; ⌘Z rend ce qui a été tapé.
- Listes personnalisées : celles d'Excel (jours, mois) et les vôtres, utilisées par la poignée de recopie et par le tri.
- Le ruban suit la largeur de la fenêtre comme dans Excel : un groupe passe sur trois lignes, puis en icônes, puis en un seul bouton.
- Fenêtre étroite : le ruban replie ses groupes un à un (boutons sans libellé, puis un seul bouton qui ouvre le groupe), dans les cinq langues.
- Affichages personnalisés : enregistrer et rappeler lignes et colonnes masquées, filtre, mise en page, zoom et sélection.

**Saisie et sélection**
- Saisie directe, F2, double-clic, barre de formule ; Entrée / Tab / Échap ; reconnaissance des nombres, pourcentages, montants, dates et heures.
- Formules : références relatives / absolues, autres feuilles, plages colorées pendant la saisie, sélection des références à la souris ou au clavier, fermeture automatique des parenthèses.
- Sélection à la souris et au clavier (Maj, ⌘ pour les sélections multiples, ⌘+flèches, lignes et colonnes entières), Atteindre et Sélectionner les cellules (formules, vides, antécédents…).
- Copier / couper / coller, menu du bouton Coller et boîte Collage spécial : formules, valeurs, formats, largeurs de colonnes, opérations (ajouter, multiplier…), blancs non compris, transposer, coller avec liaison. Échange avec les autres applications en texte tabulé.
- Rechercher et remplacer : dans la feuille ou le classeur, dans les formules ou les valeurs, casse, cellule entière, Remplacer tout en une seule annulation, Rechercher tout avec sa liste de résultats.
- Image de cellules : Coller ▸ Image, Image liée (elle suit ses cellules, comme l'appareil photo d'Excel), Copier comme image.
- Liens hypertexte : page web, fichier, emplacement du classeur, adresse de courrier ; clic pour suivre, création automatique à la saisie d'une adresse.
- Poignée de recopie avec séries (nombres, dates, « Article 1 », jours, mois), double-clic pour recopier vers le bas, ⌘D / ⌘R.
- Annuler / rétablir sur toutes les opérations.

**Calcul**
- 337 fonctions : maths, statistiques, logique, texte, date et heure, recherche (RECHERCHEV, RECHERCHEX, INDEX, EQUIV…), finance, ingénierie.
- Tableaux dynamiques avec propagation (`=A1:A3*2`, `A1#`, FILTRE, TRIER, UNIQUE, SEQUENCE…) et erreur `#PROPAGATION!`.
- Noms définis (classeur ou feuille), références 3D (`Feuil1:Feuil3!A1`), références structurées (`Tableau1[Colonne]`), intersection et union.
- Recalcul par dépendances : seules les formules qui lisent une cellule modifiée sont recalculées. Modes Automatique et Manuel, Calculer maintenant (F9), Calculer la feuille (Maj+F9), indicateur « Calculer » dans la barre d'état.
- Références circulaires : avertissement d'Excel, cellule signalée dans la barre d'état, liste des références circulaires ; calcul itératif réglable dans les préférences (nombre d'itérations, écart maximal).
- Outils de formules : Gestionnaire de noms, Créer à partir de la sélection, Insérer une fonction et Arguments de la fonction, Afficher les formules, flèches d'antécédents et de dépendants, Vérification des erreurs, Évaluer la formule, Fenêtre Espion.

**Mise en forme**
- Police, taille, gras, italique, souligné, barré, couleurs de police et de remplissage, bordures, alignements, retrait, renvoi à la ligne, fusion.
- Formats de nombre Excel complets (codes personnalisés, sections, couleurs, dates, fractions, scientifique, comptabilité).
- Mise en forme conditionnelle : règles de surbrillance, valeurs hautes / basses, barres de données, nuances de couleurs, 20 jeux d'icônes, règles par formule, gestionnaire de règles.
- Styles de cellules : galerie des 49 styles d'Excel, styles personnalisés (nouveau, modifier, dupliquer, supprimer).
- Thèmes de document : les 29 thèmes d'Excel, jeux de couleurs et de polices, personnalisation ; les couleurs et polices « du thème » suivent un changement de thème dans les cellules, tableaux, graphiques et formes.
- Largeur / hauteur par glisser, ajustement automatique, masquer / afficher.

**Données**
- Filtres automatiques (valeurs, texte, nombres, dates, couleurs, 10 premiers), tri à plusieurs niveaux avec listes personnalisées, suppression des doublons.
- Tableaux structurés : 60 styles, ligne de total, colonnes calculées, agrandissement automatique à la saisie.
- Tableaux croisés dynamiques : volet des champs avec glisser-déposer, filtres, regroupements, « afficher les valeurs comme », styles, détail par double-clic, champs et éléments calculés, ordre de résolution.
- Segments (tableaux croisés et tableaux) et chronologies : sélection au clic ou par glisser, sélection multiple, connexions de rapport, styles ; les éléments sans données sont grisés.
- Graphiques croisés dynamiques : ils suivent la disposition, les filtres et l'actualisation du tableau croisé, avec leurs boutons de champ.
- Validation des données : tous les critères, liste déroulante dans la cellule, message de saisie, alertes, cercles autour des données non valides.
- Outils de données : Convertir (assistant en 3 étapes), remplissage instantané, filtre avancé, consolider (avec liaison aux données source).
- Plan : grouper / dissocier lignes et colonnes, niveaux et boutons + / − dans la marge, plan automatique, sous-total avec ses 11 fonctions.
- Analyse de scénarios : valeur cible, gestionnaire de scénarios avec feuille de synthèse, table de données.
- Import d'un fichier texte ou CSV, mémorisé dans le classeur et relu par Actualiser tout.

**Graphiques, images et formes**
- 37 types de graphiques : histogrammes, barres, courbes, aires, secteurs, anneau, nuage de points, bulles, radar, combinés, et leurs variantes 3D.
- Création depuis la sélection, sélection de chaque élément au clic, volet Format, axes secondaires, échelles manuelles, étiquettes, courbes de tendance, barres d'erreur, Sélectionner des données, Modifier le type de graphique.
- Disposition libre : titre, légende, zone de traçage, titres d'axes et étiquettes se déplacent à la souris ; mise en forme du texte de chaque élément ; couleur, épaisseur et tirets des quadrillages.
- Images (fichier, glisser-déposer, collage) et 15 formes avec texte : déplacement, redimensionnement, rotation, remplissage, contour, flèches.
- Objets : sélection multiple (Maj ou ⌘ + clic, Sélectionner les objets), aligner, répartir, grouper / dissocier, volet Sélection (renommer, masquer, réordonner) ; graphiques, images et formes partagent un seul ordre d'empilement.
- Images : rogner (marques, forme, proportions, remplir, ajuster), luminosité, contraste, saturation, nuance, recoloration, transparence, rétablir ; 26 styles d'image, bordure, reflet, biseau, rotation 3D, ombres à flou réel ; 15 effets artistiques (crayon, craie, flou, mosaïque, photocopie…) ; compresser les images.
- Effets : ombre (9 préréglages), lueur, contours adoucis des images.
- Onglet Dessin : stylos, crayon, surligneur, gomme, sélection au lasso, conversion de l'encre en forme, relecture.
- WordArt : galerie de 15 styles, remplissage (uni ou dégradé), contour, ombre et reflet du texte, les 40 transformations d'Excel (arche, cercle, vague, gonflé…) avec leur poignée de réglage, plusieurs tailles dans une même forme.
- Symboles : comme Excel pour Mac, Insertion ▸ Symbole ouvre le visualiseur de caractères de macOS ; le caractère choisi va dans la cellule, la zone de texte ou la note en cours.
- Icônes (63 pictogrammes, recherche, catégories) et images SVG, avec remplissage et contour du graphique.
- SmartArt : 8 dispositions (liste, processus, cycle, hiérarchie, relation, matrice, pyramide…), volet Texte, couleurs, styles, conversion en formes.
- Équations : 9 équations prédéfinies, structures (fractions, indices, racines, intégrales, matrices…), saisie au format linéaire avec aperçu.
- Les graphiques suivent les données en direct, les insertions de lignes et les renommages de feuilles.
- Graphiques sparkline dans les cellules : courbe, histogramme, positif / négatif, 36 styles, points marqués, axes.

![Neuf des types de graphiques](docs/captures/types-de-graphiques.jpg)

Un classeur d'exemple (133 graphiques sur 8 feuilles) se génère avec :

```bash
cargo run -p opensheet_io --example chart_gallery -- galerie.xlsx
```

**Révision**
- Notes (bulles jaunes) et commentaires en fil avec réponses et résolution, volet Commentaires, @mentions (enregistrées comme dans Excel).
- Orthographe (F7) avec le correcteur de macOS : suggestions, ignorer, ajouter au dictionnaire, langue ; cellules, notes, commentaires, formes, titres de graphiques, en-têtes et pieds de page.
- Dictionnaire des synonymes (⇧F7) et traduction, avec ce que macOS fournit sur la machine, sans service en ligne.
- Vérification d'accessibilité : volet des erreurs, avertissements et conseils avec les actions recommandées, indicateur dans la barre d'état.
- Statistiques du classeur.
- Protection de la feuille (15 autorisations, mot de passe, cellules verrouillées ou masquées) et de la structure du classeur.

**Partage et aide**
- Partager ▸ Envoyer une copie : le classeur ou un PDF, par la feuille de partage de macOS (Mail, Messages, AirDrop…).
- Volet Aide avec recherche (ce document et la liste des raccourcis clavier), envoi de commentaires.

**Automatisation**
- Scripts Office en TypeScript (onglet Automatiser) : un script `function main(workbook: ExcelScript.Workbook)` s'exécute sur le classeur ouvert, en une seule étape d'annulation ; un script qui échoue ne change rien.
- Modèle objet `ExcelScript` : classeur, feuilles (dont masquer et afficher), plages (valeurs, formules, formats de nombre, tri, recherche, remplissage, copie, insertion et suppression, plusieurs zones), mise en forme, tableaux et filtres, noms, volets figés, graphiques, mise en forme conditionnelle, validation, commentaires et notes, tableaux croisés dynamiques, formes et images, protection de feuille. Un membre absent s'arrête avec son nom et sa ligne.
- Éditeur de code : couleurs, indentation, complétion (membres et énumérations d'`ExcelScript`, avec leur signature), aide sur les paramètres, rechercher et remplacer (⌘F), aller à la ligne (⌘L), problèmes soulignés pendant la frappe — y compris un nom de membre inconnu ou un mauvais nombre d'arguments, signalés avant l'exécution —, sortie de `console.log` avec la ligne, erreur placée sur sa ligne. Six exemples prêts à exécuter.
- Enregistrer les actions : ce que vous faites dans le classeur devient un script (saisie, formats, lignes et colonnes, recopie, tri, filtres, tableaux, noms, feuilles, volets, graphiques, collage, validation, notes, mise en forme conditionnelle simple) ; une action qui ne peut pas être écrite est listée comme non enregistrée.
- Les scripts sont des fichiers `.osts`, le format d'Excel, dans un dossier au choix (Préférences ▸ Général) ; importer, exporter, renommer, dupliquer.
- Un script tourne dans un bac à sable : pas d'accès aux fichiers ni au réseau, arrêt au bout de 120 s.
- Classeurs avec macros (`.xlsm`, `.xltm`) : le projet VBA est conservé à l'octet près à l'enregistrement ; boîte Macro et lecture du code, en lecture seule.

**Mise en page et impression**
- Marges, orientation, papier, zone d'impression, sauts de page, titres à répéter, mise à l'échelle, en-têtes et pieds de page, arrière-plan de feuille.
- Vue Mise en page et Aperçu des sauts de page ; aperçu avant impression.
- Impression vers une imprimante (choix de l'imprimante, copies, pages, recto verso), vers un fichier PDF (polices incorporées) ou vers Aperçu.

**Structure et fichiers**
- Insérer / supprimer lignes et colonnes (formules réécrites), feuilles multiples (ajouter, renommer, dupliquer, déplacer, couleur d'onglet, masquer et afficher).
- Ouvrir et enregistrer en `.xlsx` : valeurs, formules, styles et styles nommés, largeurs, fusions, volets figés, noms, graphiques, tableaux, filtres, mise en forme conditionnelle, validation, protection, notes et commentaires, mise en page, tableaux croisés dynamiques, images et formes, liens hypertexte, scénarios, tables de données, consolidation, imports de texte, plan, sparklines, thème du document, segments et chronologies, champs calculés, graphiques croisés, groupes d'objets, rognage et effets, disposition des graphiques, arrière-plan, encre, affichages personnalisés, WordArt, images SVG, SmartArt, équations. Les fichiers produits s'ouvrent dans Excel sans réparation.
- Ouvrir et enregistrer en `.xlsm`, `.xltx` et `.xltm` : le projet VBA et ses signatures sont gardés tels quels.
- Ouvrir et enregistrer en `.csv` et `.tsv`.

## Ce qui manque

**Hors périmètre** : l'exécution des macros VBA (leur code est conservé et lisible ; l'automatisation passe par les scripts Office).

**Pas encore faits** (les boutons sont dans le ruban et affichent « pas encore disponible ») :
- L'enregistrement d'une macro VBA (l'enregistreur de scripts, lui, existe : Automatiser ▸ Enregistrer les actions).
- Co-édition en direct, invitation de personnes et lien de partage : ils demandent un service en ligne qu'OpenSheet n'a pas.

**Partiels**
- Graphiques : une étiquette de données ne se sélectionne pas seule ; la zone de traçage des graphiques 3D ne se déplace pas ; ombre et lueur pour la zone de graphique et la zone de traçage seulement ; pas de contour ni d'effets pour le texte des graphiques.
- Tableaux croisés : pas de jeux (OLAP) ; les boutons de champ d'un graphique croisé n'ont pas de liste déroulante ; avec plusieurs champs de lignes, le graphique croisé n'a pas d'axe à plusieurs niveaux.
- Formes et images : un graphique n'entre pas dans un groupe ; les membres d'un groupe ne se sélectionnent pas un par un ; les effets artistiques sont nos propres filtres (ils ressemblent à ceux d'Excel sans être identiques) et sept d'entre eux manquent (ceux qui reposent sur les textures d'Office) ; deux styles d'image à double cadre manquent ; biseau du dessus seulement ; pas de suppression de l'arrière-plan ; reflet, biseau et rotation 3D pour les images seulement.
- WordArt : pas de biseau ni de lueur du texte ; pendant la saisie, le texte d'une forme déformée s'affiche à plat.
- Icônes : jeu de pictogrammes propre au projet (celui d'Office n'est pas libre) ; pas de conversion en forme.
- SmartArt : le texte se modifie dans le volet Texte, pas dans les formes ; pas d'onglet Format SmartArt. Un SmartArt créé ou modifié dans OpenSheet s'ouvre dans Excel comme un groupe de formes d'aspect identique (OpenSheet, lui, le relit comme un SmartArt) ; un SmartArt venu d'Excel et non modifié est réécrit tel quel.
- Équations : pas de curseur dans l'équation construite, la saisie se fait au format linéaire ; à l'impression, le texte linéaire.
- Dessin : l'encre est enregistrée sous forme de formes libres (Excel la rouvre comme des formes) et reste au-dessus des autres objets.
- Scripts Office : la vérification avant exécution suit les types d'`ExcelScript` mais se tait dès qu'un type n'est pas certain (`any`, paramètre sans annotation, résultat de `filter`/`map`) ; pas de membres pour les segments, la protection du classeur ni les connecteurs coudés ; pas de `fetch`. Un script qui change la protection d'une feuille ne peut pas être annulé, comme à la main.
- Enregistreur d'actions : ne sait pas écrire les fils de commentaires, les tableaux croisés, les formes et images, les segments, les sparklines, le plan, la mise en page, les liens, ni les mises en forme conditionnelles autres que « valeur de cellule » et « formule ».
- Classeurs avec macros : les contrôles de formulaire, les contrôles ActiveX et les feuilles de dialogue ne sont pas lus, donc pas réécrits ; un bouton affecté à une macro est perdu à l'enregistrement.
- Préférences : les volets absents d'OpenSheet sont grisés (graphique, saisie semi-automatique, enregistrement, compatibilité, sécurité, ruban) ; pas de style de référence L1C1.
- Correction automatique : la liste de départ est courte (signes et quelques fautes courantes par langue), sans boîte d'exceptions.
- Révision ▸ Autoriser la modification des plages : pas encore.
- Impression : seuls les préréglages « par défaut » et « derniers utilisés » existent ; ceux de macOS ne sont pas lus.
- @mentions : sans annuaire d'organisation, la liste propose les personnes déjà présentes dans le classeur, l'utilisateur, ou une adresse saisie en entier ; aucun message n'est envoyé à la personne mentionnée.
- Synonymes : macOS n'en fournit qu'en anglais. Traduction : macOS 26 ou plus récent, langues téléchargées dans Réglages Système. Orthographe : pas de correction automatique ; les formes dans un groupe ou un SmartArt ne sont pas vérifiées.
- Accessibilité : les graphiques ne sont pas contrôlés (ils n'ont pas de texte de remplacement).
- Insérer une fonction : les descriptions des fonctions sont lues dans une installation d'Excel présente sur la machine ; sans elle, seule la syntaxe est affichée.
- Rechercher et remplacer : la boîte bloque la fenêtre tant qu'elle est ouverte (celle d'Excel reste à côté).
- Collage spécial : un collage transposé ne reporte ni les commentaires ni les liens hypertexte. Image liée : sa formule ne se modifie pas dans la barre de formule.
- Obtenir des données : fichiers texte et CSV seulement (pas de bases de données, de web ni de Power Query).
- Sparklines : pas d'axe de dates. Plan : pas de styles automatiques ; la marge du plan n'apparaît pas en vue Mise en page.
- Thèmes : les effets sont enregistrés mais ne changent pas le dessin.
- Calcul : un fichier enregistré en mode manuel est recalculé une fois à l'ouverture.
- Ruban : quelques grands boutons hors rangée (Trier et filtrer, Rechercher et sélectionner, Audit de formules) passent directement en icône ; les onglets contextuels se replient encore de droite à gauche.

### Vitesse du recalcul

Mesures en mode release sur un classeur de 100 000 formules :

| Modification | Durée |
|---|---|
| Une cellule lue par deux formules, ou par aucune | 0,04 ms |
| Une cellule lue par toutes les formules | 51 ms |
| Premier calcul de 200 000 formules | environ 105 ms |

Le recalcul redevient complet dans quatre cas : feuille ajoutée, supprimée, renommée ou déplacée ; nom défini ou tableau modifié ; plus de la moitié des cellules changées d'un coup ; classeur contenant une référence circulaire.

Pour refaire les mesures :

```bash
cargo run -p opensheet_engine --release --example bench
```

## Architecture

```
crates/
  engine/   moteur de calcul, sans dépendance d'interface
            valeurs, analyseur de formules (localisé), évaluateur, 337 fonctions,
            formats de nombre, modèle de classeur, annuler/rétablir, opérations d'édition,
            et un module par fonctionnalité (filtres, tableaux, mise en forme conditionnelle,
            validation, commentaires, protection, mise en page, tableaux croisés, graphiques, formes)
  io/       lecture / écriture .xlsx et .csv
  print/    pagination et écriture PDF
  script/   scripts Office : TypeScript compilé par oxc, exécuté par QuickJS dans un bac à sable,
            modèle objet ExcelScript branché sur les opérations du moteur
  app/      l'application GPUI : ruban, grille peinte directement, éditeur de cellule,
            une interface par fonctionnalité dans src/features/
```

Les formules sont stockées sous forme canonique (noms anglais) et traduites à l'affichage et à la saisie ; un fichier écrit en français s'ouvre donc tel quel en anglais.

Tous les tests :

```bash
cargo test -p opensheet_engine -p opensheet_io -p opensheet_print -p opensheet_script -p opensheet
```

## Licence

MIT ou Apache-2.0, au choix.

OpenSheet est un projet indépendant, sans lien avec Microsoft. Excel est une marque de Microsoft Corporation.
