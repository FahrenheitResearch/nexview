use crate::render::map::MapView;

pub struct City {
    pub name: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub population: u32,
}

/// ~100 major US cities: all state capitals + cities over 200k population.
pub const US_CITIES: &[City] = &[
    // State capitals (marked with population; some overlap with large cities)
    City { name: "Montgomery", lat: 32.3668, lon: -86.3000, population: 200603 },
    City { name: "Juneau", lat: 58.3005, lon: -134.4197, population: 32255 },
    City { name: "Phoenix", lat: 33.4484, lon: -112.0740, population: 1608139 },
    City { name: "Little Rock", lat: 34.7465, lon: -92.2896, population: 202591 },
    City { name: "Sacramento", lat: 38.5816, lon: -121.4944, population: 524943 },
    City { name: "Denver", lat: 39.7392, lon: -104.9903, population: 715522 },
    City { name: "Hartford", lat: 41.7658, lon: -72.6734, population: 121054 },
    City { name: "Dover", lat: 39.1582, lon: -75.5244, population: 39403 },
    City { name: "Tallahassee", lat: 30.4383, lon: -84.2807, population: 196169 },
    City { name: "Atlanta", lat: 33.7490, lon: -84.3880, population: 498715 },
    City { name: "Honolulu", lat: 21.3069, lon: -157.8583, population: 350964 },
    City { name: "Boise", lat: 43.6150, lon: -116.2023, population: 235684 },
    City { name: "Springfield", lat: 39.7817, lon: -89.6501, population: 114394 },
    City { name: "Indianapolis", lat: 39.7684, lon: -86.1581, population: 887642 },
    City { name: "Des Moines", lat: 41.5868, lon: -93.6250, population: 214133 },
    City { name: "Topeka", lat: 39.0473, lon: -95.6752, population: 126587 },
    City { name: "Frankfort", lat: 38.2009, lon: -84.8733, population: 28602 },
    City { name: "Baton Rouge", lat: 30.4515, lon: -91.1871, population: 227470 },
    City { name: "Augusta", lat: 44.3106, lon: -69.7795, population: 19136 },
    City { name: "Annapolis", lat: 38.9784, lon: -76.4922, population: 40812 },
    City { name: "Boston", lat: 42.3601, lon: -71.0589, population: 675647 },
    City { name: "Lansing", lat: 42.7325, lon: -84.5555, population: 112644 },
    City { name: "St. Paul", lat: 44.9537, lon: -93.0900, population: 311527 },
    City { name: "Jackson", lat: 32.2988, lon: -90.1848, population: 153701 },
    City { name: "Jefferson City", lat: 38.5767, lon: -92.1735, population: 43079 },
    City { name: "Helena", lat: 46.5884, lon: -112.0245, population: 32315 },
    City { name: "Lincoln", lat: 40.8136, lon: -96.7026, population: 291082 },
    City { name: "Carson City", lat: 39.1638, lon: -119.7674, population: 58639 },
    City { name: "Concord", lat: 43.2081, lon: -71.5376, population: 43976 },
    City { name: "Trenton", lat: 40.2171, lon: -74.7429, population: 90871 },
    City { name: "Santa Fe", lat: 35.6870, lon: -105.9378, population: 89117 },
    City { name: "Albany", lat: 42.6526, lon: -73.7562, population: 99224 },
    City { name: "Raleigh", lat: 35.7796, lon: -78.6382, population: 467665 },
    City { name: "Bismarck", lat: 46.8083, lon: -100.7837, population: 73529 },
    City { name: "Columbus", lat: 39.9612, lon: -82.9988, population: 905748 },
    City { name: "Oklahoma City", lat: 35.4676, lon: -97.5164, population: 681054 },
    City { name: "Salem", lat: 44.9429, lon: -123.0351, population: 175535 },
    City { name: "Harrisburg", lat: 40.2732, lon: -76.8867, population: 50099 },
    City { name: "Providence", lat: 41.8240, lon: -71.4128, population: 190934 },
    City { name: "Columbia", lat: 34.0007, lon: -81.0348, population: 136632 },
    City { name: "Pierre", lat: 44.3683, lon: -100.3510, population: 14091 },
    City { name: "Nashville", lat: 36.1627, lon: -86.7816, population: 689447 },
    City { name: "Austin", lat: 30.2672, lon: -97.7431, population: 978908 },
    City { name: "Salt Lake City", lat: 40.7608, lon: -111.8910, population: 200133 },
    City { name: "Montpelier", lat: 44.2601, lon: -72.5754, population: 8074 },
    City { name: "Richmond", lat: 37.5407, lon: -77.4360, population: 226610 },
    City { name: "Olympia", lat: 47.0379, lon: -122.9007, population: 55605 },
    City { name: "Charleston", lat: 38.3498, lon: -81.6326, population: 48006 },
    City { name: "Madison", lat: 43.0731, lon: -89.4012, population: 269840 },
    City { name: "Cheyenne", lat: 41.1400, lon: -104.8202, population: 65132 },

    // Large cities > 200k (not already listed as capitals)
    City { name: "New York", lat: 40.7128, lon: -74.0060, population: 8336817 },
    City { name: "Los Angeles", lat: 34.0522, lon: -118.2437, population: 3979576 },
    City { name: "Chicago", lat: 41.8781, lon: -87.6298, population: 2693976 },
    City { name: "Houston", lat: 29.7604, lon: -95.3698, population: 2304580 },
    City { name: "San Antonio", lat: 29.4241, lon: -98.4936, population: 1547253 },
    City { name: "San Diego", lat: 32.7157, lon: -117.1611, population: 1423851 },
    City { name: "Dallas", lat: 32.7767, lon: -96.7970, population: 1304379 },
    City { name: "San Jose", lat: 37.3382, lon: -121.8863, population: 1013240 },
    City { name: "Jacksonville", lat: 30.3322, lon: -81.6557, population: 949611 },
    City { name: "Fort Worth", lat: 32.7555, lon: -97.3308, population: 918915 },
    City { name: "Charlotte", lat: 35.2271, lon: -80.8431, population: 874579 },
    City { name: "San Francisco", lat: 37.7749, lon: -122.4194, population: 873965 },
    City { name: "Seattle", lat: 47.6062, lon: -122.3321, population: 737015 },
    City { name: "Washington DC", lat: 38.9072, lon: -77.0369, population: 689545 },
    City { name: "El Paso", lat: 31.7619, lon: -106.4850, population: 678815 },
    City { name: "Detroit", lat: 42.3314, lon: -83.0458, population: 639111 },
    City { name: "Memphis", lat: 35.1495, lon: -90.0490, population: 633104 },
    City { name: "Portland", lat: 45.5152, lon: -122.6784, population: 652503 },
    City { name: "Las Vegas", lat: 36.1699, lon: -115.1398, population: 641903 },
    City { name: "Louisville", lat: 38.2527, lon: -85.7585, population: 633045 },
    City { name: "Baltimore", lat: 39.2904, lon: -76.6122, population: 585708 },
    City { name: "Milwaukee", lat: 43.0389, lon: -87.9065, population: 577222 },
    City { name: "Albuquerque", lat: 35.0844, lon: -106.6504, population: 564559 },
    City { name: "Tucson", lat: 32.2226, lon: -110.9747, population: 542629 },
    City { name: "Fresno", lat: 36.7378, lon: -119.7871, population: 542107 },
    City { name: "Mesa", lat: 33.4152, lon: -111.8315, population: 504258 },
    City { name: "Kansas City", lat: 39.0997, lon: -94.5786, population: 508090 },
    City { name: "Omaha", lat: 41.2565, lon: -95.9345, population: 486051 },
    City { name: "Colorado Springs", lat: 38.8339, lon: -104.8214, population: 478961 },
    City { name: "Virginia Beach", lat: 36.8529, lon: -75.9780, population: 459470 },
    City { name: "Long Beach", lat: 33.7701, lon: -118.1937, population: 466742 },
    City { name: "Oakland", lat: 37.8044, lon: -122.2712, population: 433031 },
    City { name: "Minneapolis", lat: 44.9778, lon: -93.2650, population: 429954 },
    City { name: "Tampa", lat: 27.9506, lon: -82.4572, population: 398564 },
    City { name: "Tulsa", lat: 36.1540, lon: -95.9928, population: 413066 },
    City { name: "Arlington TX", lat: 32.7357, lon: -97.1081, population: 394266 },
    City { name: "New Orleans", lat: 29.9511, lon: -90.0715, population: 383997 },
    City { name: "Wichita", lat: 37.6872, lon: -97.3301, population: 397532 },
    City { name: "Cleveland", lat: 41.4993, lon: -81.6944, population: 372624 },
    City { name: "Bakersfield", lat: 35.3733, lon: -119.0187, population: 403455 },
    City { name: "Aurora CO", lat: 39.7294, lon: -104.8319, population: 386261 },
    City { name: "Anaheim", lat: 33.8366, lon: -117.9143, population: 350742 },
    City { name: "Pittsburgh", lat: 40.4406, lon: -79.9959, population: 302971 },
    City { name: "Cincinnati", lat: 39.1031, lon: -84.5120, population: 309317 },
    City { name: "Riverside", lat: 33.9533, lon: -117.3962, population: 314998 },
    City { name: "St. Louis", lat: 38.6270, lon: -90.1994, population: 301578 },
    City { name: "Stockton", lat: 37.9577, lon: -121.2908, population: 320804 },
    City { name: "Corpus Christi", lat: 27.8006, lon: -97.3964, population: 317863 },
    City { name: "Irvine", lat: 33.6846, lon: -117.8265, population: 307670 },
    City { name: "Orlando", lat: 28.5383, lon: -81.3792, population: 307573 },
    City { name: "Newark", lat: 40.7357, lon: -74.1724, population: 311549 },
    City { name: "Buffalo", lat: 42.8864, lon: -78.8784, population: 278349 },
    City { name: "Chandler", lat: 33.3062, lon: -111.8413, population: 275987 },
    City { name: "Laredo", lat: 27.5036, lon: -99.5076, population: 255205 },
    City { name: "Norfolk", lat: 36.8508, lon: -76.2859, population: 244076 },
    City { name: "Durham", lat: 35.9940, lon: -78.8986, population: 283506 },
    City { name: "Chula Vista", lat: 32.6401, lon: -117.0842, population: 275487 },
    City { name: "Lexington", lat: 38.0406, lon: -84.5037, population: 322570 },
    City { name: "Anchorage", lat: 61.2181, lon: -149.9003, population: 291247 },
    City { name: "Henderson", lat: 36.0395, lon: -114.9817, population: 320189 },
    City { name: "Greensboro", lat: 36.0726, lon: -79.7920, population: 299035 },
    City { name: "Plano", lat: 33.0198, lon: -96.6989, population: 285494 },
    City { name: "Scottsdale", lat: 33.4942, lon: -111.9261, population: 241361 },
    City { name: "Miami", lat: 25.7617, lon: -80.1918, population: 442241 },
    City { name: "Spokane", lat: 47.6588, lon: -117.4260, population: 228989 },
    City { name: "Irving", lat: 32.8140, lon: -96.9489, population: 256684 },
    City { name: "Fremont", lat: 37.5485, lon: -121.9886, population: 230504 },
    City { name: "Gilbert", lat: 33.3528, lon: -111.7890, population: 267918 },
    City { name: "San Bernardino", lat: 34.1083, lon: -117.2898, population: 222101 },
    City { name: "Birmingham", lat: 33.5186, lon: -86.8104, population: 200733 },
    City { name: "Rochester", lat: 43.1566, lon: -77.6088, population: 211328 },
    City { name: "Chesapeake", lat: 36.7682, lon: -76.2875, population: 249422 },
    City { name: "Glendale AZ", lat: 33.5387, lon: -112.1860, population: 248325 },
    City { name: "North Las Vegas", lat: 36.1989, lon: -115.1175, population: 262527 },
    City { name: "Winston-Salem", lat: 36.0999, lon: -80.2442, population: 249545 },
    City { name: "Lubbock", lat: 33.5779, lon: -101.8552, population: 263930 },
    City { name: "Jersey City", lat: 40.7178, lon: -74.0431, population: 292449 },
    City { name: "Reno", lat: 39.5296, lon: -119.8138, population: 264165 },
    City { name: "St. Petersburg", lat: 27.7676, lon: -82.6403, population: 258308 },
];

pub struct GeoOverlays;

impl GeoOverlays {
    /// Draw city dots and labels, filtering by zoom level and population.
    pub fn draw_cities(
        painter: &egui::Painter,
        map_view: &MapView,
        rect: egui::Rect,
        zoom: f64,
    ) {
        let screen_w = rect.width() as f64;
        let screen_h = rect.height() as f64;

        let min_population = if zoom < 5.0 {
            500_000
        } else if zoom <= 7.0 {
            200_000
        } else {
            0
        };

        for city in US_CITIES {
            if city.population < min_population {
                continue;
            }

            let (px, py) = map_view.lat_lon_to_pixel(city.lat, city.lon, screen_w, screen_h);
            let pos = egui::pos2(rect.left() + px as f32, rect.top() + py as f32);

            // Skip cities outside the visible rect (with some margin for label)
            if pos.x < rect.left() - 80.0
                || pos.x > rect.right() + 10.0
                || pos.y < rect.top() - 10.0
                || pos.y > rect.bottom() + 10.0
            {
                continue;
            }

            // Dot radius based on population
            let radius = if city.population >= 1_000_000 {
                4.0
            } else if city.population >= 500_000 {
                3.0
            } else {
                2.0
            };

            let dot_color = egui::Color32::from_white_alpha(220);
            painter.circle_filled(pos, radius, dot_color);

            // Label offset to the right of the dot
            let label_color = egui::Color32::from_white_alpha(200);
            painter.text(
                pos + egui::vec2(radius + 3.0, -1.0),
                egui::Align2::LEFT_CENTER,
                city.name,
                egui::FontId::proportional(10.0),
                label_color,
            );
        }
    }
}
