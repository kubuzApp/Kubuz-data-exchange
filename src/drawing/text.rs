//! The words on a sheet, in the dossier's language, and the way numbers
//! are written: millimetres to a tenth, degrees to a tenth, decimal comma
//! (ISO 129-1), no trailing zero.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Nl,
    En,
    Fr,
}

impl Lang {
    pub fn parse(code: &str) -> Result<Lang, String> {
        match code.trim().to_ascii_lowercase().as_str() {
            "" | "nl" => Ok(Lang::Nl),
            "en" => Ok(Lang::En),
            "fr" => Ok(Lang::Fr),
            other => Err(format!("language {other:?}: nl, en or fr")),
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::Nl => "nl",
            Lang::En => "en",
            Lang::Fr => "fr",
        }
    }

    /// The label for `key`, one of the names below.
    pub fn t(self, key: &str) -> &'static str {
        let i = match self {
            Lang::Nl => 0,
            Lang::En => 1,
            Lang::Fr => 2,
        };
        for (k, v) in LABELS {
            if *k == key {
                return v[i];
            }
        }
        panic!("no label {key}")
    }
}

const LABELS: &[(&str, [&str; 3])] = &[
    ("owner", ["Eigenaar", "Legal owner", "Propriétaire"]),
    ("title", ["Titel", "Title", "Titre"]),
    ("id", ["Identificatienummer", "Identification number", "Numéro d'identification"]),
    ("created", ["Gemaakt door", "Created by", "Créé par"]),
    ("approved", ["Goedgekeurd", "Approved", "Approuvé"]),
    ("designer", ["Tekenaar", "Designer", "Dessinateur"]),
    ("design_date", ["Tekendatum", "Design date", "Date du dessin"]),
    ("approved_date", ["Datum goedkeuring", "Approved date", "Date d'approbation"]),
    ("size", ["Formaat", "Size", "Format"]),
    ("weight", ["Gewicht", "Weight", "Poids"]),
    ("extents", ["Afmetingen", "Extents", "Encombrement"]),
    (
        "property",
        [
            "Deze tekening is eigendom van {} en mag niet geheel of gedeeltelijk worden gekopieerd zonder voorafgaande schriftelijke toestemming van de eigenaar",
            "This drawing is property of {} and may not be copied in whole or in part without the prior written consent of the owner",
            "Ce dessin est la propriété de {} et ne peut être copié, en tout ou en partie, sans l'accord écrit préalable du propriétaire",
        ],
    ),
    ("section", ["DOORSNEDE", "SECTION", "COUPE"]),
    (
        "chain_note",
        [
            "Kettingmaten vanaf de rand: aanslagmaten per plooilijn, gaten op hartlijn.",
            "Chain dimensions from the edge: back-gauge per bend line, holes on centre.",
            "Cotes en chaîne depuis le bord : butée par ligne de pli, trous à l'axe.",
        ],
    ),
    (
        "section_note",
        [
            "Doorsnedes: buitenmaten tot het theoretische snijpunt van de buitenvlakken; 90° niet aangeduid.",
            "Sections: outside dimensions to the theoretical intersection of the outer faces; 90° not marked.",
            "Coupes : cotes extérieures jusqu'à l'intersection théorique des faces extérieures ; 90° non coté.",
        ],
    ),
    ("date", ["Datum", "Date of issue", "Date d'émission"]),
    ("material", ["Materiaal", "Material", "Matériau"]),
    ("thickness", ["Dikte", "Thickness", "Épaisseur"]),
    ("quantity", ["Aantal", "Quantity", "Quantité"]),
    ("doctype", ["Documenttype", "Document type", "Type de document"]),
    ("detail", ["Stuktekening", "Detail drawing", "Dessin de définition"]),
    ("status", ["Status", "Document status", "Statut"]),
    ("revision", ["Revisie", "Revision", "Révision"]),
    ("scale", ["Schaal", "Scale", "Échelle"]),
    ("sheet", ["Blad", "Sheet", "Feuille"]),
    ("format", ["Formaat", "Paper size", "Format"]),
    ("tolerances", ["Algemene toleranties", "General tolerances", "Tolérances générales"]),
    ("projection", ["Projectiemethode", "Projection method", "Méthode de projection"]),
    ("blank", ["Afwikkeling L × B", "Flat pattern L × W", "Développé L × l"]),
    ("folded_size", ["Geplooid L × B × H", "Folded L × W × H", "Plié L × l × H"]),
    ("flat", ["AFWIKKELING", "FLAT PATTERN", "DÉVELOPPÉ"]),
    (
        "flat_sub",
        [
            "gezien vanaf de buitenzijde (zichtzijde)",
            "seen from the outside (show side)",
            "vu de l'extérieur (face visible)",
        ],
    ),
    ("bend", ["Plooi", "Bend", "Pli"]),
    ("angle", ["Hoek", "Angle", "Angle"]),
    ("direction", ["Richting", "Direction", "Sens"]),
    ("inside_r", ["R binnen", "Inside R", "R intérieur"]),
    ("allowance", ["Toeslag", "Allowance", "Allongement"]),
    ("up", ["OP", "UP", "HAUT"]),
    ("down", ["NEER", "DOWN", "BAS"]),
    ("mm", ["Maten in mm", "Dimensions in mm", "Cotes en mm"]),
    ("deburr", ["Kanten ontbraamd", "Edges deburred", "Arêtes ébavurées"]),
    (
        "bend_note",
        [
            "Plooihoek = afwijking t.o.v. vlak. OP = naar de kijker toe, NEER = van de kijker af.",
            "Bend angle = deviation from flat. UP = towards the viewer, DOWN = away from the viewer.",
            "Angle de pli = écart par rapport au plat. HAUT = vers l'observateur, BAS = à l'opposé.",
        ],
    ),
    (
        "dim_note",
        [
            "Lopende maten vanaf de hoek linksonder.",
            "Running dimensions from the lower-left corner.",
            "Cotes cumulées depuis le coin inférieur gauche.",
        ],
    ),
    ("no_bends", ["Geen plooien", "No bends", "Pas de plis"]),
    (
        "minor_note",
        [
            "Ontlastingen, inkepingen en kleine rondingen: niet bemaat, volgens DXF.",
            "Reliefs, notches and small rounds: not dimensioned, per DXF.",
            "Dégagements, encoches et petits arrondis : non cotés, selon DXF.",
        ],
    ),
    (
        "curve_note",
        [
            "Vrije vorm: alleen eind- en uiterste punten bemaat, verloop volgens DXF.",
            "Free-form contour: only its ends and extreme points are dimensioned, shape per DXF.",
            "Contour libre : seuls les extrémités et points extrêmes sont cotés, forme selon DXF.",
        ],
    ),
    ("size", ["Afmeting", "Size", "Encombrement"]),
    ("isometric", ["ISOMETRIE", "ISOMETRIC", "ISOMÉTRIE"]),
    ("view", ["AANZICHT", "VIEW", "VUE"]),
    (
        "outside_note",
        [
            "Buitenmaten, lopend vanaf de hoek linksonder van elk aanzicht.",
            "Outside dimensions, running from the lower-left corner of each view.",
            "Cotes extérieures, cumulées depuis le coin inférieur gauche de chaque vue.",
        ],
    ),
    (
        "radius_note",
        ["Binnenradius plooien", "Inside bend radius", "Rayon intérieur des plis"],
    ),
    (
        "angle_note",
        [
            "Plooihoek = afwijking t.o.v. vlak; 90° niet aangeduid. OP = naar de buitenhuid toe.",
            "Bend angle = deviation from flat; 90° not marked. UP = towards the outer skin.",
            "Angle de pli = écart par rapport au plat ; 90° non coté. HAUT = vers la peau extérieure.",
        ],
    ),
    (
        "minor_note_folded",
        [
            "Ontlastingen en inkepingen aan de plooien: niet bemaat, volgens afwikkeling.",
            "Bend reliefs and notches: not dimensioned, per flat pattern.",
            "Dégagements et encoches aux plis : non cotés, selon le développé.",
        ],
    ),
    ("dossier", ["Tekeningendossier", "Drawing dossier", "Dossier de dessins"]),
    ("dossier_caps", ["TEKENINGENDOSSIER", "DRAWING DOSSIER", "DOSSIER DE DESSINS"]),
    ("cover", ["VOORBLAD", "COVER SHEET", "PAGE DE GARDE"]),
    ("cover_title", ["Voorblad", "Cover sheet", "Page de garde"]),
    ("assembly", ["SAMENSTELLING", "ASSEMBLY", "ENSEMBLE"]),
    ("assembly_title", ["Samenstelling en stuklijst", "Assembly and parts list", "Ensemble et nomenclature"]),
    ("bom", ["STUKLIJST", "PARTS LIST", "NOMENCLATURE"]),
    ("bom_title", ["Stuklijst", "Parts list", "Nomenclature"]),
    ("laser_list", ["LASERLIJST", "LASER LIST", "LISTE LASER"]),
    ("laser_title", ["Laserlijst", "Laser list", "Liste laser"]),
    ("bend_list", ["PLOOILIJST", "BEND LIST", "LISTE DE PLIAGE"]),
    ("bend_title", ["Plooilijst", "Bend list", "Liste de pliage"]),
    ("continued", ["vervolg", "continued", "suite"]),
    ("contents", ["Inhoud", "Contents", "Contenu"]),
    ("pos", ["Pos", "Pos", "Pos"]),
    ("part", ["Stuk", "Part", "Pièce"]),
    ("name", ["Benaming", "Description", "Désignation"]),
    ("number", ["Tekeningnummer", "Drawing number", "Numéro de dessin"]),
    ("customer", ["Klant", "Customer", "Client"]),
    ("parts_total", ["Stuks in totaal", "Parts in total", "Pièces au total"]),
    ("unique_parts", ["Unieke stukken", "Unique parts", "Pièces uniques"]),
    ("total_weight", ["Totaal gewicht", "Total weight", "Poids total"]),
    ("total", ["Totaal", "Total", "Total"]),
    ("cut_length", ["Snijlengte", "Cut length", "Long. de coupe"]),
    ("holes", ["Gaten", "Holes", "Trous"]),
    ("area", ["Opp. dm²", "Area dm²", "Surface dm²"]),
    ("per_piece", ["kg/stuk", "kg each", "kg/pièce"]),
    ("kg_total", ["kg totaal", "kg total", "kg total"]),
    ("bend_length", ["Plooilengte", "Bend length", "Longueur de pli"]),
    ("bends", ["Plooien", "Bends", "Plis"]),
    ("bends_total", ["plooien in totaal", "bends in total", "plis au total"]),
    ("tool", ["Gereedschap", "Tool", "Outil"]),
    ("pieces", ["stuks", "pcs", "pièces"]),
    ("mirror_of", ["Spiegelbeeld van pos {}", "Mirror image of pos {}", "Image miroir de la pos {}"]),
    ("route_laser", ["Laser", "Laser", "Laser"]),
    ("route_bend", ["Laser + plooien", "Laser + bending", "Laser + pliage"]),
    ("route", ["Route", "Route", "Gamme"]),
    ("folded_part", ["geplooid stuk", "folded part", "pièce pliée"]),
    (
        "order_note",
        [
            "Stuktekeningen in productievolgorde: eerst enkel laser, dan laser en plooien; daarbinnen per materiaal en dikte.",
            "Detail drawings in production order: laser only first, then laser and bending; within that by material and thickness.",
            "Dessins de définition dans l'ordre de production : laser seul d'abord, puis laser et pliage ; ensuite par matériau et épaisseur.",
        ],
    ),
    (
        "balloon_note",
        [
            "Positienummers volgens de stuklijst; elke rij linkt naar het blad van het stuk.",
            "Item numbers as in the parts list; every row links to the part's sheet.",
            "Repères selon la nomenclature ; chaque ligne renvoie à la feuille de la pièce.",
        ],
    ),
    (
        "laser_note",
        [
            "Per materiaal en dikte. Afwikkeling L × B gezien vanaf de zichtzijde; snijlengte = omtrek + gaten; oppervlak netto.",
            "Per material and thickness. Flat pattern L × W seen from the show side; cut length = outline + holes; net area.",
            "Par matériau et épaisseur. Développé L × l vu de la face visible ; longueur de coupe = contour + trous ; surface nette.",
        ],
    ),
    (
        "bend_list_note",
        [
            "Per dikte en binnenradius (gereedschap). Hoek = afwijking t.o.v. vlak; OP = naar de zichtzijde; plooilengte langs de plooilijn.",
            "Per thickness and inside radius (tooling). Angle = deviation from flat; UP = towards the show side; bend length along the bend line.",
            "Par épaisseur et rayon intérieur (outillage). Angle = écart par rapport au plat ; HAUT = vers la face visible ; longueur le long du pli.",
        ],
    ),
    ("page_of", ["blad", "sheet", "feuille"]),
    ("pages_range", ["bladen", "sheets", "feuilles"]),

    ("details", ["Stuktekeningen", "Detail drawings", "Dessins de définition"]),
];

/// A length in mm to a tenth, decimal comma, no trailing zero: `1000`,
/// `1001,6`, `0,5`. Never `-0`.
pub fn mm(v: f64) -> String {
    tenth(v)
}

/// An angle in degrees to a tenth with the degree sign: `90°`, `45,5°`.
pub fn deg(v: f64) -> String {
    format!("{}°", tenth(v))
}

fn tenth(v: f64) -> String {
    let t = (v * 10.0).round() as i64;
    let (sign, t) = if t < 0 { ("-", -t) } else { ("", t) };
    if t % 10 == 0 {
        format!("{sign}{}", t / 10)
    } else {
        format!("{sign}{},{}", t / 10, t % 10)
    }
}

/// The ISO 5455 scale as written: `1:5`, `2:1`.
pub fn scale(s: f64) -> String {
    if s >= 1.0 {
        format!("{}:1", (s).round() as i64)
    } else {
        format!("1:{}", (1.0 / s).round() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_round_to_a_tenth_with_a_decimal_comma() {
        assert_eq!(mm(1000.0), "1000");
        assert_eq!(mm(1001.6493), "1001,6");
        assert_eq!(mm(45.546_546), "45,5");
        assert_eq!(mm(36.0), "36");
        assert_eq!(mm(0.04), "0");
        assert_eq!(mm(-0.04), "0");
        assert_eq!(deg(90.0), "90°");
        assert_eq!(deg(135.04), "135°");
        assert_eq!(scale(0.2), "1:5");
        assert_eq!(scale(2.0), "2:1");
    }

    #[test]
    fn every_label_exists_in_every_language() {
        for (k, _) in LABELS {
            for l in [Lang::Nl, Lang::En, Lang::Fr] {
                assert!(!l.t(k).is_empty());
            }
        }
        assert_eq!(Lang::parse("").unwrap().code(), "nl");
        assert!(Lang::parse("de").is_err());
    }
}
