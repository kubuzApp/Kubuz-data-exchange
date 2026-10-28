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
