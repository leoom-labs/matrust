//! isoutlier, filloutliers and rmoutliers. Expected numbers come from
//! independent NumPy/SciPy implementations of the published definitions
//! (numpy.quantile method "hazen", scipy.special.erfcinv for the MAD constant,
//! scipy.stats.t.ppf for the Grubbs and generalized ESD critical values):
//! bash tool/ref-python.sh tool/ref/outliers.py prints the module below.
//! No MATLAB evidence.
use matrust::{Array, Engine};

#[rustfmt::skip]
mod reference {
    // MAD constant 1.4826022185056018
    pub const X: &[f64] = &[10.909, 10.437, 8.287, 10.814, 10.786, 25.0, 9.874, 10.21, 9.482, 10.181, 10.612, 12.118, f64::NAN, 9.819, 9.96, 8.976, 9.246, -3.0, 9.219, 10.762, 9.09, 11.22, 13.2, 10.099, 10.947, 8.688, 10.487, 10.556, 10.67, 10.115, 15.5, 9.534, 11.002, 10.961, 8.849, 10.364, 10.1, 9.023, 10.129, 10.516];
    pub const MEDIAN_TF: &[bool] = &[false, false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false, false, true, false, false, false, false, false, false, false, false, false];
    pub const MEDIAN_L: &[f64] = &[7.2032827008706395];
    pub const MEDIAN_U: &[f64] = &[13.216717299129362];
    pub const MEDIAN_C: &[f64] = &[10.21];
    pub const MEDIAN_K2_TF: &[bool] = &[false, false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false, true, false, false, false, false, true, false, false, false, false, false, false, false, true, false, false, false, false, false, false, false, false, false];
    pub const MEDIAN_K2_L: &[f64] = &[8.205521800580428];
    pub const MEDIAN_K2_U: &[f64] = &[12.214478199419574];
    pub const MEDIAN_K2_C: &[f64] = &[10.21];
    pub const MEAN_TF: &[bool] = &[false, false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false];
    pub const MEAN_L: &[f64] = &[0.01976060225230114];
    pub const MEAN_U: &[f64] = &[20.736239397747703];
    pub const MEAN_C: &[f64] = &[10.378000000000002];
    pub const QUARTILES_TF: &[bool] = &[false, false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false, true, false, false, false, false, true, false, false, false, false, false, false, false, true, false, false, false, false, false, false, false, false, false];
    pub const QUARTILES_L: &[f64] = &[7.5269999999999975];
    pub const QUARTILES_U: &[f64] = &[12.775000000000002];
    pub const QUARTILES_C: &[f64] = &[10.151];
    pub const PERCENTILES_TF: &[bool] = &[false, false, true, false, false, true, false, false, false, false, false, false, false, false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false, false, true, false, false, false, false, false, false, false, false, false];
    pub const PERCENTILES_L: &[f64] = &[8.467450000000001];
    pub const PERCENTILES_U: &[f64] = &[14.464999999999993];
    pub const PERCENTILES_C: &[f64] = &[11.466224999999998];
    pub const GRUBBS_TF: &[bool] = &[false, false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false, true, false, false, false, false, true, false, false, false, false, false, false, false, true, false, false, false, false, false, false, false, false, false];
    pub const GRUBBS_L: &[f64] = &[7.632616650306604];
    pub const GRUBBS_U: &[f64] = &[12.598354778264826];
    pub const GRUBBS_C: &[f64] = &[10.115485714285715];
    pub const GESD_TF: &[bool] = &[false, false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false, true, false, false, false, false, true, false, false, false, false, false, false, false, true, false, false, false, false, false, false, false, false, false];
    pub const GESD_L: &[f64] = &[7.632616650306604];
    pub const GESD_U: &[f64] = &[12.598354778264826];
    pub const GESD_C: &[f64] = &[10.115485714285715];
    pub const GESD_MAX1_TF: &[bool] = &[false, false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false];
    pub const GESD_MAX1_L: &[f64] = &[2.4199709894318815];
    pub const GESD_MAX1_U: &[f64] = &[17.5664500631997];
    pub const GESD_MAX1_C: &[f64] = &[9.99321052631579];
    pub const GESD_A20_TF: &[bool] = &[false, false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false, true, false, false, false, false, true, false, false, false, false, false, false, false, true, false, false, false, false, false, false, false, false, false];
    pub const GESD_A20_L: &[f64] = &[7.9231629950378375];
    pub const GESD_A20_U: &[f64] = &[12.30780843353359];
    pub const GESD_A20_C: &[f64] = &[10.115485714285715];
    pub const GESD_HUGE_TF: &[bool] = &[false, false, false, true, false, true, false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false, false, false, false, true, false, false, false, false, false, false, false, false, false, false, false, false, false, false];
    pub const GESD_HUGE_L: &[f64] = &[2.229513702401859];
    pub const GESD_HUGE_U: &[f64] = &[17.749229154741002];
    pub const GESD_HUGE_C: &[f64] = &[9.989371428571431];
    pub const GRUBBS_HUGE_TF: &[bool] = &[false, false, false, true, false, true, false, false, false, false, true, false, false, false, false, false, false, true, false, false, false, false, true, false, false, true, false, false, false, false, true, false, false, false, false, false, false, false, false, false];
    pub const GRUBBS_HUGE_L: &[f64] = &[7.712809137713554];
    pub const GRUBBS_HUGE_U: &[f64] = &[12.532690862286445];
    pub const GRUBBS_HUGE_C: &[f64] = &[10.12275];
    pub const Y: &[f64] = &[1.0, 1.2, 0.9, 1.1, 6.0, 1.0, 1.3, 0.8, 1.1, -3.0, 1.2, 1.0, f64::NAN, 1.1, 0.9, 1.4, 5.0, 1.0];
    pub const MOVMED5_TF: &[bool] = &[false, false, false, false, true, false, false, false, false, true, false, false, false, false, false, false, true, false];
    pub const MOVMED5_L: &[f64] = &[0.5552193344483196, 0.6052193344483197, 0.6552193344483191, 0.6552193344483191, 0.2104386688966392, 0.2104386688966392, 0.2104386688966392, 0.11043866889663911, 0.2104386688966392, 0.11043866889663911, 0.6052193344483197, 0.6052193344483197, 0.6052193344483197, 0.6052193344483193, 0.1380483361207987, 0.21043866889663865, 0.08804833612079865, -0.37912266220672186];
    pub const MOVMED5_U: &[f64] = &[1.4447806655516804, 1.4947806655516804, 1.5447806655516811, 1.5447806655516811, 1.9895613311033609, 1.9895613311033609, 1.9895613311033609, 1.8895613311033608, 1.9895613311033609, 1.8895613311033608, 1.4947806655516804, 1.4947806655516804, 1.4947806655516804, 1.4947806655516809, 2.3619516638792013, 1.9895613311033615, 2.311951663879201, 3.179122662206722];
    pub const MOVMED5_C: &[f64] = &[1.0, 1.05, 1.1, 1.1, 1.1, 1.1, 1.1, 1.0, 1.1, 1.0, 1.05, 1.05, 1.05, 1.05, 1.25, 1.1, 1.2, 1.4];
    pub const MOVMED4_TF: &[bool] = &[false, false, false, false, true, false, false, false, false, true, false, false, false, false, false, false, true, false];
    pub const MOVMED4_L: &[f64] = &[0.6552193344483197, 0.5552193344483196, 0.6052193344483197, 0.4828290016724792, 0.6052193344483193, 0.5328290016724788, 0.03804833612079861, 0.38282900167247913, -0.16195166387920124, 0.06043866889663918, 0.6052193344483197, 0.11043866889663911, 0.6552193344483201, 0.5552193344483196, 0.21043866889663865, 0.1380483361207987, 0.08804833612079865, -0.37912266220672186];
    pub const MOVMED4_U: &[f64] = &[1.5447806655516805, 1.4447806655516804, 1.4947806655516804, 1.8171709983275206, 1.4947806655516809, 1.8671709983275215, 2.261951663879201, 1.717170998327521, 2.0619516638792015, 1.839561331103361, 1.4947806655516804, 1.8895613311033608, 1.54478066555168, 1.4447806655516804, 1.9895613311033615, 2.3619516638792013, 2.311951663879201, 3.179122662206722];
    pub const MOVMED4_C: &[f64] = &[1.1, 1.0, 1.05, 1.15, 1.05, 1.2000000000000002, 1.15, 1.05, 0.9500000000000001, 0.9500000000000001, 1.05, 1.0, 1.1, 1.0, 1.1, 1.25, 1.2, 1.4];
    pub const MOVMED21_TF: &[bool] = &[false, false, false, false, true, false, false, false, false, true, false, false, false, false, false, false, true, false];
    pub const MOVMED21_L: &[f64] = &[0.6552193344483197, 0.5552193344483196, 0.6052193344483197, 0.4828290016724792, 0.6052193344483193, 0.5328290016724788, 0.03804833612079861, 0.38282900167247913, -0.16195166387920124, 0.06043866889663918, 0.6052193344483197, 0.11043866889663911, 0.6552193344483201, 0.5552193344483196, 0.21043866889663865, 0.1380483361207987, 0.08804833612079865, -0.37912266220672186];
    pub const MOVMED21_U: &[f64] = &[1.5447806655516805, 1.4447806655516804, 1.4947806655516804, 1.8171709983275206, 1.4947806655516809, 1.8671709983275215, 2.261951663879201, 1.717170998327521, 2.0619516638792015, 1.839561331103361, 1.4947806655516804, 1.8895613311033608, 1.54478066555168, 1.4447806655516804, 1.9895613311033615, 2.3619516638792013, 2.311951663879201, 3.179122662206722];
    pub const MOVMED21_C: &[f64] = &[1.1, 1.0, 1.05, 1.15, 1.05, 1.2000000000000002, 1.15, 1.05, 0.9500000000000001, 0.9500000000000001, 1.05, 1.0, 1.1, 1.0, 1.1, 1.25, 1.2, 1.4];
    pub const MOVMEAN5_TF: &[bool] = &[false, false, false, false, true, false, false, false, false, true, false, false, false, false, false, false, false, false];
    pub const MOVMEAN5_L: &[f64] = &[0.8042045485855415, 0.8563508326896292, -1.2847932266533508, -1.2847932266533508, -1.2512308889595727, -1.291553691597961, -1.291553691597961, -2.490247241551578, -2.4846428340745934, -2.4891511585734745, -3.0024380578656658, -3.0024380578656658, 0.8563508326896292, 0.7759629650796072, -0.8163333142835367, -0.7512069473912539, -0.8678939838193287, -0.8378756617049947];
    pub const MOVMEAN5_U: &[f64] = &[1.2624621180811255, 1.243649167310371, 5.364793226653351, 5.364793226653351, 5.371230888959573, 5.371553691597962, 5.371553691597962, 2.9702472415515775, 3.044642834074594, 2.929151158573475, 3.152438057865666, 3.152438057865666, 1.243649167310371, 1.424037034920393, 5.016333314283537, 4.511206947391254, 5.017893983819329, 5.771208995038329];
    pub const MOVMEAN5_C: &[f64] = &[1.0333333333333334, 1.05, 2.04, 2.04, 2.06, 2.04, 2.04, 0.23999999999999985, 0.28, 0.22000000000000003, 0.07500000000000001, 0.07500000000000001, 1.05, 1.1, 2.1, 1.8800000000000001, 2.075, 2.466666666666667];
    pub const MOVMEAN4_TF: &[bool] = &[false, false, false, false, true, false, false, true, false, true, false, false, false, false, false, false, true, false];
    pub const MOVMEAN4_L: &[f64] = &[0.9302943725152287, 0.8500303055350998, 0.8950806661517035, -0.6637813684548326, -0.7515995735607373, -0.5738331005719184, -0.7151839408303959, 0.8002000800640638, -2.4024273689550926, -2.4035798319182344, -2.3869504462925324, -3.109763569693364, 0.9800000000000001, 0.88, 0.8313399559225034, -0.23306665142682936, -0.2793151870554631, -0.17696719603066224];
    pub const MOVMEAN4_U: &[f64] = &[1.2697056274847713, 1.216636361131567, 1.2049193338482966, 5.263781368454833, 5.251599573560737, 5.273833100571919, 5.265183940830397, 1.2997999199359358, 2.502427368955093, 2.4535798319182343, 2.5369504462925327, 2.576430236360031, 1.22, 1.12, 1.4353267107441632, 4.433066651426829, 4.4293151870554635, 5.110300529363996];
    pub const MOVMEAN4_C: &[f64] = &[1.1, 1.0333333333333334, 1.05, 2.3, 2.25, 2.35, 2.2750000000000004, 1.0499999999999998, 0.050000000000000044, 0.025000000000000022, 0.07500000000000001, -0.26666666666666666, 1.1, 1.0, 1.1333333333333333, 2.1, 2.075, 2.466666666666667];
    pub const TY: &[f64] = &[0.0, 1.06, 2.3200000000000003, 3.6000000000000005, 4.500000000000001, 5.670000000000001, 6.320000000000001, 7.400000000000001, 8.110000000000001, 9.340000000000002, 10.05, 11.5, 12.81, 14.02, 14.87, 15.54, 16.11, 17.259999999999998];
    pub const MOVMED_SP_TF: &[bool] = &[false, false, false, false, true, false, false, false, false, true, false, false, false, false, false, false, true, false];
    pub const MOVMED_SP_L: &[f64] = &[0.6552193344483197, 0.5552193344483196, 0.6552193344483201, 0.21043866889663865, 0.6552193344483191, 0.03804833612079861, 0.11043866889663911, 0.38282900167247913, -0.5343419966550418, 0.6552193344483201, 0.11043866889663911, 0.6552193344483197, 0.8276096672241595, 0.21043866889663865, 0.1380483361207987, 0.21043866889663865, 0.08804833612079865, -0.37912266220672186];
    pub const MOVMED_SP_U: &[f64] = &[1.5447806655516805, 1.4447806655516804, 1.54478066555168, 1.9895613311033615, 1.5447806655516811, 2.261951663879201, 1.8895613311033608, 1.717170998327521, 2.134341996655042, 1.54478066555168, 1.8895613311033608, 1.5447806655516805, 1.2723903327758406, 1.9895613311033615, 2.3619516638792013, 1.9895613311033615, 2.311951663879201, 3.179122662206722];
    pub const MOVMED_SP_C: &[f64] = &[1.1, 1.0, 1.1, 1.1, 1.1, 1.15, 1.0, 1.05, 0.8, 1.1, 1.0, 1.1, 1.05, 1.1, 1.25, 1.1, 1.2, 1.4];
    pub const MOVMEAN_SP_TF: &[bool] = &[false, false, false, false, true, false, true, false, false, false, false, false, false, false, false, false, true, false];
    pub const MOVMEAN_SP_L: &[f64] = &[0.9302943725152287, 0.8500303055350998, 0.883363638868433, -0.7995127862757223, -0.5738331005719184, -0.7151839408303959, 0.8002000800640638, -2.4024273689550926, -2.4035798319182344, -3.109163648216324, -3.109763569693364, 0.9302943725152287, 0.9651471862576143, 0.8313399559225034, -0.23306665142682936, -0.2793151870554631, -0.2793151870554631, -0.3941125496954281];
    pub const MOVMEAN_SP_U: &[f64] = &[1.2697056274847713, 1.216636361131567, 1.2499696944649001, 6.132846119609056, 5.273833100571919, 5.265183940830397, 1.2997999199359358, 2.502427368955093, 2.4535798319182343, 2.642496981549657, 2.576430236360031, 1.2697056274847713, 1.134852813742386, 1.4353267107441632, 4.433066651426829, 4.4293151870554635, 4.4293151870554635, 6.3941125496954285];
    pub const MOVMEAN_SP_C: &[f64] = &[1.1, 1.0333333333333334, 1.0666666666666667, 2.6666666666666665, 2.35, 2.2750000000000004, 1.0499999999999998, 0.050000000000000044, 0.025000000000000022, -0.2333333333333333, -0.26666666666666666, 1.1, 1.05, 1.1333333333333333, 2.1, 2.075, 2.075, 3.0];
    pub const M: &[f64] = &[1.458, 1.219, 8.0, -1.21, 1.015, -0.646, 0.197, 0.07, 0.758, -1.086, 0.084, -0.053, -0.518, 1.265, -7.5, -0.642, -0.512, 0.337, -1.047, 0.57, 1.7, 9.0, 0.772, -1.111, -0.343, 0.574, -0.147, -1.248, -0.219, -0.527, 0.27, 6.0, -1.284, 0.849, 0.526, -1.669];
    pub const M_TF: &[bool] = &[false, false, true, false, false, false, false, false, false, false, false, false, false, false, true, false, false, false, false, false, false, true, false, false, false, false, false, false, false, false, false, true, false, false, false, false];
    pub const M_L: &[f64] = &[-3.553403427406707, -4.1327801531925346, -3.4432422784938184];
    pub const M_C: &[f64] = &[0.1405, -0.0875, -0.183];
    pub const M_DIM2_TF: &[bool] = &[true, false, true, false, true, false, false, false, false, false, true, true, false, false, false, true, false, true, true, false, false, true, false, false, false, true, false, false, false, false, false, true, true, false, false, false];
    pub const M_DIM2_U: &[f64] = &[1.2928313398326088, 1.4056885239718613, 7.8710553666696095, -0.693896544912671, 0.9050507319920735, 0.25782595521487733, 0.5468641628346377, 5.502198385799508, 1.9167504347627533, 8.273731732489496, 0.8092887160745224, -0.12354265762281258];
    pub const M_RM1: &[bool] = &[false, false, true, false, false, false, false, true, false, true, false, false];
    pub const M_RM2: &[bool] = &[false, false, true, false, false, false, false, false, false, false, false, false];
    pub const Z: &[f64] = &[3.0, 3.2, 2.9, 12.0, 3.1, 3.0, -6.0, 3.3, f64::NAN, 3.1, 2.8, 20.0];
    pub const ZMED_TF: &[bool] = &[false, false, false, true, false, false, true, false, false, false, false, true];
    pub const ZMED_L: &[f64] = &[2.21043866889664];
    pub const ZMED_U: &[f64] = &[3.98956133110336];
    pub const ZMED_C: &[f64] = &[3.1];
    pub const FILL_CENTER: &[f64] = &[3.0, 3.2, 2.9, 3.1, 3.1, 3.0, 3.1, 3.3, f64::NAN, 3.1, 2.8, 3.1];
    pub const FILL_CLIP: &[f64] = &[3.0, 3.2, 2.9, 3.98956133110336, 3.1, 3.0, 2.21043866889664, 3.3, f64::NAN, 3.1, 2.8, 3.98956133110336];
    pub const FILL_VALUE: &[f64] = &[3.0, 3.2, 2.9, 0.0, 3.1, 3.0, 0.0, 3.3, f64::NAN, 3.1, 2.8, 0.0];
    pub const FILL_LINEAR: &[f64] = &[3.0, 3.2, 2.9, 3.0, 3.1, 3.0, 3.15, 3.3, f64::NAN, 3.1, 2.8, 2.4999999999999996];
    pub const FILL_PREVIOUS: &[f64] = &[3.0, 3.2, 2.9, 2.9, 3.1, 3.0, 3.0, 3.3, f64::NAN, 3.1, 2.8, 2.8];
    pub const FILL_NEXT: &[f64] = &[3.0, 3.2, 2.9, 3.1, 3.1, 3.0, 3.3, 3.3, f64::NAN, 3.1, 2.8, 20.0];
    pub const TA: &[f64] = &[1.0, 1.1, 0.9, 1.0, 9.0, 1.2, 1.0, 0.8, 1.1, 1.0];
    pub const TB: &[f64] = &[5.0, 5.1, 4.9, 5.2, 5.0, 5.1, -20.0, 5.0, 4.8, 5.1];
    pub const TC: &[f64] = &[0.0, 100.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    pub const T_RM_AB: &[bool] = &[false, false, false, false, true, false, true, false, false, false];
    pub const T_RM_A: &[bool] = &[false, false, false, false, true, false, false, false, false, false];
}
use reference::*;

/// MATLAB text of a numeric vector (NaN as NaN, shortest round-trip digits).
fn row(v: &[f64]) -> String {
    let items: Vec<String> = v
        .iter()
        .map(|x| {
            if x.is_nan() {
                "NaN".into()
            } else {
                format!("{x}")
            }
        })
        .collect();
    format!("[{}]", items.join(" "))
}

fn run(code: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(code)
        .unwrap_or_else(|err| panic!("{code}: {err}"));
    e
}

fn values(e: &Engine, name: &str) -> (usize, usize, Vec<f64>) {
    match e.array(name).unwrap_or_else(|| panic!("missing {name}")) {
        Array::Numeric(m) => (m.rows(), m.cols(), m.data().to_vec()),
        Array::Logical(l) => (
            l.rows(),
            l.cols(),
            l.data().iter().map(|&b| f64::from(b)).collect(),
        ),
        other => panic!("{name}: unexpected class {}", other.class()),
    }
}

fn scalar(e: &Engine, name: &str) -> f64 {
    values(e, name).2[0]
}

fn close(actual: &[f64], expected: &[f64], rel: f64, what: &str) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{what}: {actual:?} vs {expected:?}"
    );
    for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
        let ok = if b.is_nan() {
            a.is_nan()
        } else {
            (a - b).abs() <= rel * b.abs() + 1e-13
        };
        assert!(ok, "{what}[{i}]: {a} vs {b}");
    }
}

fn flags(e: &Engine, name: &str, rows: usize, cols: usize, expected: &[bool], what: &str) {
    let (r, c, v) = match e.array(name) {
        Some(Array::Logical(l)) => (l.rows(), l.cols(), l.data().to_vec()),
        other => panic!("{what}: {name} is not logical: {other:?}"),
    };
    assert_eq!((r, c), (rows, cols), "{what}: {name} shape");
    let v: Vec<bool> = v.iter().map(|&b| b != 0).collect();
    assert_eq!(v, expected, "{what}: {name}");
}

/// Compare `[TF, L, U, C] = isoutlier(...)` with a reference case.
#[allow(clippy::too_many_arguments)]
fn check(
    setup: &str,
    call: &str,
    tf: &[bool],
    lo: &[f64],
    hi: &[f64],
    center: &[f64],
    shape: (usize, usize),
    what: &str,
) {
    let e = run(&format!("{setup} [TF,L,U,C]={call};"));
    flags(&e, "TF", shape.0, shape.1, tf, what);
    for (name, expected) in [("L", lo), ("U", hi), ("C", center)] {
        close(
            &values(&e, name).2,
            expected,
            1e-10,
            &format!("{what} {name}"),
        );
    }
}

#[test]
fn whole_vector_methods_match_the_numpy_reference() {
    let setup = format!("x={};", row(X));
    for (call, tf, lo, hi, c) in [
        ("isoutlier(x)", MEDIAN_TF, MEDIAN_L, MEDIAN_U, MEDIAN_C),
        (
            "isoutlier(x,'median','ThresholdFactor',2)",
            MEDIAN_K2_TF,
            MEDIAN_K2_L,
            MEDIAN_K2_U,
            MEDIAN_K2_C,
        ),
        ("isoutlier(x,'mean')", MEAN_TF, MEAN_L, MEAN_U, MEAN_C),
        (
            "isoutlier(x,'quartiles')",
            QUARTILES_TF,
            QUARTILES_L,
            QUARTILES_U,
            QUARTILES_C,
        ),
        (
            "isoutlier(x,'percentiles',[5 95])",
            PERCENTILES_TF,
            PERCENTILES_L,
            PERCENTILES_U,
            PERCENTILES_C,
        ),
        (
            "isoutlier(x,'grubbs')",
            GRUBBS_TF,
            GRUBBS_L,
            GRUBBS_U,
            GRUBBS_C,
        ),
        ("isoutlier(x,'gesd')", GESD_TF, GESD_L, GESD_U, GESD_C),
        (
            "isoutlier(x,'gesd','MaxNumOutliers',1)",
            GESD_MAX1_TF,
            GESD_MAX1_L,
            GESD_MAX1_U,
            GESD_MAX1_C,
        ),
        (
            "isoutlier(x,\"gesd\",\"ThresholdFactor\",0.2)",
            GESD_A20_TF,
            GESD_A20_L,
            GESD_A20_U,
            GESD_A20_C,
        ),
    ] {
        check(&setup, call, tf, lo, hi, c, (1, X.len()), call);
    }
    // Dominant outliers: thresholds still follow the small spread of the rest.
    let mut huge = X.to_vec();
    (huge[3], huge[10], huge[25]) = (1e12, -5e11, 3e6);
    let setup_huge = format!("x={};", row(&huge));
    for (call, tf, lo, hi, c) in [
        (
            "isoutlier(x,'gesd')",
            GESD_HUGE_TF,
            GESD_HUGE_L,
            GESD_HUGE_U,
            GESD_HUGE_C,
        ),
        (
            "isoutlier(x,'grubbs')",
            GRUBBS_HUGE_TF,
            GRUBBS_HUGE_L,
            GRUBBS_HUGE_U,
            GRUBBS_HUGE_C,
        ),
    ] {
        check(&setup_huge, call, tf, lo, hi, c, (1, X.len()), call);
    }
    // A column gives a column of flags; NaN is never an outlier.
    let e = run(&format!("{setup} tf=isoutlier(x'); n=tf(13);"));
    flags(&e, "tf", X.len(), 1, MEDIAN_TF, "column");
    assert_eq!(scalar(&e, "n"), 0.);
}

#[test]
fn moving_methods_match_the_numpy_reference() {
    let setup = format!("y={}; ty={};", row(Y), row(TY));
    for (call, tf, lo, hi, c) in [
        (
            "isoutlier(y,'movmedian',5)",
            MOVMED5_TF,
            MOVMED5_L,
            MOVMED5_U,
            MOVMED5_C,
        ),
        (
            "isoutlier(y,'movmedian',4)",
            MOVMED4_TF,
            MOVMED4_L,
            MOVMED4_U,
            MOVMED4_C,
        ),
        (
            "isoutlier(y,'movmedian',[2 1])",
            MOVMED21_TF,
            MOVMED21_L,
            MOVMED21_U,
            MOVMED21_C,
        ),
        (
            "isoutlier(y,'movmean',5,'ThresholdFactor',1.5)",
            MOVMEAN5_TF,
            MOVMEAN5_L,
            MOVMEAN5_U,
            MOVMEAN5_C,
        ),
        (
            "isoutlier(y,'movmean',4,'ThresholdFactor',1.2)",
            MOVMEAN4_TF,
            MOVMEAN4_L,
            MOVMEAN4_U,
            MOVMEAN4_C,
        ),
        (
            "isoutlier(y,'movmedian',3.5,'SamplePoints',ty)",
            MOVMED_SP_TF,
            MOVMED_SP_L,
            MOVMED_SP_U,
            MOVMED_SP_C,
        ),
        (
            "isoutlier(y,'movmean',[1.5 2],'SamplePoints',ty,'ThresholdFactor',1.2)",
            MOVMEAN_SP_TF,
            MOVMEAN_SP_L,
            MOVMEAN_SP_U,
            MOVMEAN_SP_C,
        ),
    ] {
        check(&setup, call, tf, lo, hi, c, (1, Y.len()), call);
    }
    // Sample points 1:n give the index windows.
    let e = run(&format!(
        "{setup} a=isoutlier(y,'movmedian',4); b=isoutlier(y,'movmedian',4,'SamplePoints',1:numel(y)); ok=isequal(a,b);"
    ));
    assert_eq!(scalar(&e, "ok"), 1.);
}

#[test]
fn matrices_work_per_column_and_along_dim_2() {
    let setup = format!("M=reshape({},12,3);", row(M));
    let e = run(&format!("{setup} [TF,L,U,C]=isoutlier(M);"));
    flags(&e, "TF", 12, 3, M_TF, "columns");
    let (r, c, l) = values(&e, "L");
    assert_eq!((r, c), (1, 3));
    close(&l, M_L, 1e-10, "column L");
    close(&values(&e, "C").2, M_C, 1e-10, "column C");
    let e = run(&format!(
        "{setup} [TF,L,U]=isoutlier(M,'mean',2,'ThresholdFactor',1);"
    ));
    flags(&e, "TF", 12, 3, M_DIM2_TF, "rows");
    let (r, c, u) = values(&e, "U");
    assert_eq!((r, c), (12, 1));
    close(&u, M_DIM2_U, 1e-10, "row U");
    // Moving thresholds have the size of the data; single data give single results.
    let e = run(&format!(
        "{setup} [TF,L]=isoutlier(M,'movmedian',5); [r,c]=size(L); s=class(isoutlier(single(M))); [~,Ls]=isoutlier(single(M)); cs=class(Ls);"
    ));
    assert_eq!((scalar(&e, "r"), scalar(&e, "c")), (12., 3.));
    // An all-NaN column has NaN statistics and no outliers.
    let e = run("[TF,L,U,C]=isoutlier([1 NaN;2 NaN;30 NaN;2 NaN]); t=TF(:,2)'; c=C(2);");
    assert_eq!(values(&e, "t").2, vec![0.; 4]);
    assert!(scalar(&e, "c").is_nan());
}

#[test]
fn single_and_integer_data_keep_their_class() {
    let e = run(
        "x=[2 3 2 40 3 2]; [~,L]=isoutlier(single(x)); a=class(L); b=class(filloutliers(int16(x),'linear')); c=class(rmoutliers(uint8(x))); B=double(filloutliers(int16(x),'linear'));",
    );
    for (name, class) in [("a", "single"), ("b", "int16"), ("c", "uint8")] {
        match e.array(name) {
            Some(Array::Character(t)) => {
                assert_eq!(String::from_utf16_lossy(t.units()), class, "{name}")
            }
            other => panic!("{name}: {other:?}"),
        }
    }
    close(
        &values(&e, "B").2,
        &[2., 3., 2., 2.5f64.round(), 3., 2.],
        0.,
        "int16 fill",
    );
}

#[test]
fn fill_methods_match_the_numpy_reference() {
    let setup = format!("z={};", row(Z));
    for (method, expected) in [
        ("'center'", FILL_CENTER),
        ("'clip'", FILL_CLIP),
        ("0", FILL_VALUE),
        ("'linear'", FILL_LINEAR),
        ("'previous'", FILL_PREVIOUS),
        ("'next'", FILL_NEXT),
    ] {
        let e = run(&format!("{setup} [B,TF,L,U,C]=filloutliers(z,{method});"));
        close(&values(&e, "B").2, expected, 1e-12, method);
        flags(&e, "TF", 1, Z.len(), ZMED_TF, method);
        close(&values(&e, "L").2, ZMED_L, 1e-12, method);
        close(&values(&e, "U").2, ZMED_U, 1e-12, method);
        close(&values(&e, "C").2, ZMED_C, 1e-12, method);
    }
    // Cubic fills are interp1 over the non-outlier, non-NaN points.
    for method in ["spline", "pchip", "makima", "nearest"] {
        let e = run(&format!(
            "{setup} B=filloutliers(z,'{method}'); tf=isoutlier(z); k=find(~tf & ~isnan(z)); i=find(tf); r=interp1(k,z(k),i,'{method}','extrap'); ok=isequal(B(i),r); kept=isequal(B(~tf),z(~tf)) || isequaln(B(~tf),z(~tf));"
        ));
        assert_eq!(scalar(&e, "ok"), 1., "{method}");
        assert_eq!(scalar(&e, "kept"), 1., "{method}");
    }
    // Moving detection with a fill, and known locations.
    let e = run(
        "B=filloutliers([1 1.2 0.9 6 1.1 1 0.8],'previous','movmedian',3); m=logical([0 1 0 0 0 1 0]); B2=filloutliers([1 2 3 4 5 6 7],-1,'OutlierLocations',m);",
    );
    close(
        &values(&e, "B").2,
        &[1., 1.2, 0.9, 0.9, 1.1, 1., 0.8],
        0.,
        "movmedian previous",
    );
    close(
        &values(&e, "B2").2,
        &[1., -1., 3., 4., 5., -1., 7.],
        0.,
        "locations",
    );
    // Fill along dim 2 of a matrix.
    let e = run("B=filloutliers([1 2 1 2 40 1; 3 3 3 3 3 3],'next',2);");
    close(
        &values(&e, "B").2,
        &[1., 3., 2., 3., 1., 3., 2., 3., 1., 3., 1., 3.],
        0.,
        "dim 2",
    );
}

#[test]
fn rmoutliers_removes_rows_elements_and_table_rows() {
    let setup = format!("M=reshape({},12,3);", row(M));
    for (min, expected) in [(1, M_RM1), (2, M_RM2)] {
        let e = run(&format!(
            "{setup} [B,TFrm,TFo]=rmoutliers(M,'MinNumOutliers',{min}); ok=isequal(B,M(~TFrm,:)); tfo=isequal(TFo,isoutlier(M));"
        ));
        flags(
            &e,
            "TFrm",
            12,
            1,
            expected,
            &format!("MinNumOutliers {min}"),
        );
        assert_eq!(scalar(&e, "ok"), 1.);
        assert_eq!(scalar(&e, "tfo"), 1.);
    }
    // Along dim 2 columns go; a vector loses elements.
    let e = run(&format!(
        "{setup} [B,TFrm]=rmoutliers(M',2); ok=isequal(B,M(~TFrm',:)'); [r,c]=size(TFrm); x={}; v=rmoutliers(x); okv=isequaln(v,x(~isoutlier(x)));",
        row(X)
    ));
    assert_eq!(scalar(&e, "ok"), 1.);
    assert_eq!((scalar(&e, "r"), scalar(&e, "c")), (1., 12.));
    assert_eq!(scalar(&e, "okv"), 1.);
    // Tables: DataVariables choose the checked variables; L, U, C are tables.
    let setup = format!(
        "T=table({}',{}',{}','VariableNames',{{'a','b','c'}});",
        row(TA),
        row(TB),
        row(TC)
    );
    let e = run(&format!(
        "{setup} [R,TFrm]=rmoutliers(T,'DataVariables',{{'a','b'}}); h=height(R); [R1,TF1]=rmoutliers(T,'DataVariables','a'); [R3,TF3,TFo,L]=rmoutliers(T); n3=height(R3); lc=class(L); lw=width(L);"
    ));
    flags(&e, "TFrm", TA.len(), 1, T_RM_AB, "table a,b");
    flags(&e, "TF1", TA.len(), 1, T_RM_A, "table a");
    let removed = T_RM_AB.iter().filter(|&&b| b).count();
    assert_eq!(scalar(&e, "h"), (TA.len() - removed) as f64);
    assert_eq!(scalar(&e, "n3"), (TA.len() - removed - 1) as f64);
    assert_eq!(scalar(&e, "lw"), 3.);
}

#[test]
fn tables_and_timetables() {
    let setup = format!(
        "T=table({}',{}',{{'p';'q';'r';'s';'t';'u';'v';'w';'x';'y'}},'VariableNames',{{'a','b','s'}});",
        row(TA),
        row(TB)
    );
    let e = run(&format!(
        "{setup} TF=isoutlier(T); TT=isoutlier(T,'OutputFormat','tabular'); k=class(TT); tb=TT.b'; F=filloutliers(T,'previous'); fb=F.b'; fs=isequal(F.s,T.s); [~,L,U,C]=isoutlier(T,'DataVariables',[1 2]); ca=C.a;"
    ));
    let a = values(&e, "TF");
    assert_eq!((a.0, a.1), (10, 3));
    let tb: Vec<bool> = values(&e, "tb").2.iter().map(|&v| v != 0.).collect();
    let expected_b: Vec<bool> = T_RM_AB
        .iter()
        .zip(T_RM_A)
        .map(|(&ab, &a)| ab && !a)
        .collect();
    assert_eq!(tb, expected_b);
    let fb = values(&e, "fb").2;
    assert_eq!(fb[6], TB[5], "previous fill in a table");
    assert_eq!(scalar(&e, "fs"), 1.);
    // A SamplePoints variable gives the positions and is not checked itself.
    let e = run(&format!(
        "y={}'; t={}'; T=table(t,y); TF=isoutlier(T,'movmedian',3.5,'SamplePoints','t'); ref=isoutlier(y,'movmedian',3.5,'SamplePoints',t); ok=isequal(TF(:,2),ref) && ~any(TF(:,1));",
        row(Y),
        row(TY)
    ));
    assert_eq!(scalar(&e, "ok"), 1.);
    // Timetables use their row times; windows are durations.
    let e = run(
        "tt=timetable(seconds([1;2;3;4;5;6]),[1;1.1;9;1;1.2;1],'VariableNames',{'X'}); a=isoutlier(tt,'movmedian',seconds(3))'; b=isoutlier([1 1.1 9 1 1.2 1],'movmedian',3); ok=isequal(a,b);",
    );
    assert_eq!(scalar(&e, "ok"), 1.);
    let mut e = Engine::default();
    let err = e
        .execute("tt=timetable(seconds([1;2;3]),[1;2;3]); isoutlier(tt,'movmedian',3)")
        .unwrap_err();
    assert!(err.contains("duration"), "{err}");
}

#[test]
fn invalid_calls_are_explicit() {
    for (code, needle) in [
        ("isoutlier([1 2 3],'medain')", "unknown method"),
        ("isoutlier('abc')", "numeric"),
        ("isoutlier({1,2})", "numeric"),
        ("isoutlier([1 2 3],'movmedian',0)", "window"),
        ("isoutlier([1 2 3],'movmedian',-2)", "window"),
        ("isoutlier([1 2 3],'movmedian',[1 -1])", "window"),
        ("isoutlier([1 2 3],'movmean')", "window"),
        (
            "isoutlier([1 2 3],'movmedian',3,'SamplePoints',[1 2])",
            "SamplePoints",
        ),
        (
            "isoutlier([1 2 3],'movmedian',3,'SamplePoints',[1 3 2])",
            "SamplePoints",
        ),
        (
            "isoutlier([1 2 3],'grubbs','ThresholdFactor',2)",
            "between 0 and 1",
        ),
        ("isoutlier([1 2 3],'ThresholdFactor',-1)", "ThresholdFactor"),
        ("isoutlier([1 2 3],'MaxNumOutliers',2)", "gesd"),
        ("isoutlier([1 2 3],'percentiles',[90 10])", "percentiles"),
        ("isoutlier([1 2 3],'OutputFormat','tabular')", "table"),
        ("isoutlier([1 2 3],0)", "dimension"),
        ("isoutlier([1 2 3],'Bogus',1)", "unknown method"),
        ("[a,b,c,d,e]=isoutlier([1 2 3])", "too many output"),
        ("filloutliers([1 2 3])", "fill method"),
        ("filloutliers([1 2 3],'spline2')", "fill method"),
        (
            "filloutliers([1 2 3],'center','OutlierLocations',logical([0 1 0]))",
            "OutlierLocations",
        ),
        (
            "filloutliers([1 2 3],0,'median','OutlierLocations',logical([0 1 0]))",
            "OutlierLocations",
        ),
        (
            "filloutliers([1 2 3],0,'OutlierLocations',logical([0 1]))",
            "OutlierLocations",
        ),
        ("rmoutliers([1 2 3],'MinNumOutliers',0)", "MinNumOutliers"),
        ("rmoutliers([1 2 3; 4 5 6],3)", "dimension"),
        ("isoutlier(table([1;2;3]),2)", "dimension 1"),
        (
            "isoutlier(table({'a';'b'}),'DataVariables',1)",
            "not numeric",
        ),
        ("isoutlier([1 2 3],'DataVariables','a')", "table"),
    ] {
        let mut e = Engine::default();
        let err = e
            .execute(code)
            .err()
            .unwrap_or_else(|| panic!("must reject: {code}"));
        assert!(err.contains(needle), "{code}: {err}");
    }
    let mut e = Engine::default();
    e.execute("a=exist('isoutlier'); b=exist('filloutliers'); c=exist('rmoutliers');")
        .unwrap();
    for name in ["a", "b", "c"] {
        assert_eq!(scalar(&e, name), 2., "{name}");
    }
}
