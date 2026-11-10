/*
  NeoPicoBlinkophone
  J.Brett, BCIT Technology Teacher Education
  October, 2023  -- updated for arduinoFFT 2.x and FastLED 3.10
  November, 2026 -- softer bass flash, centred bars, blue/white/red colour scheme, hi-hat sparkles,
                    60 frames a second: core 1 records audio non-stop while core 0 analyses it and drives the LEDs,
                    stereo: left and right channels both recorded, spectrum shows them side by side,
                    sensitivity (or brightness) knob on A2

  Board:  Raspberry Pi Pico, using the "Raspberry Pi Pico/RP2040" core by Earle Philhower.
          Boards Manager URL:
          https://github.com/earlephilhower/arduino-pico/releases/download/global/package_rp2040_index.json
          Tutorial: https://bc-robotics.com/tutorials/programming-the-raspberry-pi-pico-with-arduino/

  Libraries (install from the Arduino Library Manager):
    arduinoFFT 2.x by Enrique Condes  (tested with 2.0.4)
    FastLED by Daniel Garcia           (tested with 3.10.9)
*/

#include <FastLED.h>
#include <arduinoFFT.h>

//------------------------------- CHANGE THESE TO MATCH YOUR LED STRANDS -------------------------------
#define NUM_LEDS    38   // LEDs on the GP0 strand
#define OFFSET     2.5   // shifts the centre of the GP0 bars. Each +1 moves it one LED up the strand. 2.5 = split between LEDs 22 and 23
#define SPECTRUM_OFFSET 2.5   // the middle for the spectrum and the newer effects. 2.5 = the middle of the blue zone below (LEDs 17-28)
                              // (when adding LEDs at the end, lower both offsets by 0.5 per LED to keep the same middle)
#define NUM_LEDS2   36   // LEDs on the second strand, on GP1
#define OFFSET2     -4   // shifts the centre of the GP1 display
#define MAX_MA    1800   // the Mini360 regulator can supply about 1800mA continuously

// The knob (the potentiometer on A2): what it controls
enum KnobJob { KNOB_SENSITIVITY, KNOB_BRIGHTNESS, KNOB_NOTHING };
const KnobJob KNOB_CONTROLS = KNOB_SENSITIVITY;
const bool    KNOB_REVERSED = false;   // set to true if the knob works the wrong way round

// Sensitivity knob: how strongly every preset reacts to the music. Middle = the tuned settings below.
// Turn it down for busy, loud songs (fewer, bigger hits) and up for calm or quiet songs (more hits, longer bars).
const float   SENS_LEAST    = 0.6f;    // knob all the way down: jumps in the music count for 0.6 times as much
const float   SENS_MOST     = 2.0f;    // knob all the way up:   jumps count for twice as much

// Brightness knob (when KNOB_CONTROLS = KNOB_BRIGHTNESS)
const uint8_t KNOB_MIN      = 5;       // brightness (0-255) with the knob turned all the way down
const uint8_t KNOB_MAX      = 255;     // brightness (0-255) with the knob turned all the way up
// Fixed brightness, used when the knob isn't controlling brightness. MAX_MA still protects the power supply
#define BRIGHTNESS 255                 // 0-255. Higher brightness = more current draw
const uint8_t FLASH_MODE_BRIGHTNESS = 255;   // the same for bass flash mode
//------------------------------------------------------------------------------------------------------

//--------------------------------- LOOK AND FEEL: tweak these to taste --------------------------------
// Colours for low, mid and high frequencies, used by the three-colour bars and the spectrum display
const CRGB  LOW_COLOUR  = CRGB(0, 0, 255);       // blue
const CRGB  MID_COLOUR  = CRGB(255, 255, 255);   // white
const CRGB  HIGH_COLOUR = CRGB(255, 0, 0);       // red
const float LOW_TOP_HZ  = 600;    // "low" is everything up to this frequency
const float MID_TOP_HZ  = 3600;   // "mid" is up to this, "high" is everything above

// Three-colour bars: grow out from the centre, blue (low) in the middle, then white (mid), then red (high)
// Each band is compared with its own recent average, so all three colours get a fair share of the strand.
// Tuned on calm, balanced and hyper songs: about 7-10 of 18 LEDs per side on average, swinging about +/-5 with the music,
// so quiet moments drop back towards the middle and big hits reach the end
const float BAR_LOW_SIZE  = 0.55f;   // how long the blue (low) bar is. Bigger = longer
const float BAR_MID_SIZE  = 0.55f;   // how long the white (mid) bar is
const float BAR_HIGH_SIZE = 0.55f;   // how long the red (high) bar is
const float BAR_GATE  = 0.8f;    // a band below this share of its recent average shows nothing. Higher = more contrast
const float BAR_QUIET = 1000;    // keeps the strand dark when nothing is playing. Measured with the stereo inputs: silence
                                 // still reads about 700 (a steady background tone); music measures several thousand
// The speed settings below are per frame, at 60 frames a second.
const float BAR_RISE = 0.46f;   // how quickly bars grow.   1.0 = instantly, smaller = smoother
const float BAR_FALL = 0.18f;   // how quickly bars shrink. 1.0 = instantly, smaller = smoother

// Spectrum display (switches GP17 + GP16). The strand is split down the middle (SPECTRUM_OFFSET):
// the higher-numbered LEDs show the RIGHT channel, the lower-numbered LEDs show the LEFT channel.
const bool HIGHER_LEDS_ARE_RIGHT = true;   // set to false to swap the two sides over
const int  WRAP_LEDS = 3;   // LEDs 1-3 carry on from LED 38 as part of the right side (the strand loops round). 0 = none
// Each colour zone below covers its own frequencies (blue = low, white = mid, red = high), and each LED
// in a zone shows its own slice of them: lowest nearest the middle, highest at the outer end.
enum Band : uint8_t { BAND_LOW, BAND_MID, BAND_HIGH };
struct SpectrumZone { int lastLed; Band band; };
const SpectrumZone SPECTRUM_ZONES[] = {   // GP0 strand, LEDs counted 1 to NUM_LEDS. Each line is the last LED of a zone.
  {  9, BAND_HIGH },   // LEDs  1-9:  red   (1-3 right, carrying on from 36-38, LED 3 = highest; 4-9 left, LED 4 = highest)
  { 16, BAND_MID  },   // LEDs 10-16: white (left)
  { 28, BAND_LOW  },   // LEDs 17-28: blue  (17-22 left, 23-28 right: lowest bass next to the middle)
  { 35, BAND_MID  },   // LEDs 29-35: white (right)
  { 38, BAND_HIGH },   // LEDs 36-38: red   (right, carrying on round to LEDs 1-3)
};                     // (the GP1 strand uses equal thirds on each side instead)
// Each LED is compared with its OWN recent average loudness, so every frequency gets a fair share of the action
// (bass and treble are very different in loudness, so a single shared volume leaves some LEDs stuck on or off).
// Tuned from 25 seconds of real audio recorded on the Pico: with 1.0 and 2.0, LEDs are lit about half the time.
const uint8_t SPECTRUM_MAX   = 255;     // brightness (0-255) of the loudest LEDs
const float   SPEC_GATE      = 1.0f;    // an LED is off until its frequency is louder than its recent average (1.0)...
const float   SPEC_FULL      = 2.0f;    // ...and full brightness at this many times its average. Closer together = more drastic
const int     SPEC_AVG_FRAMES = 120;    // how many frames the average covers (120 = 2 seconds)
const float   SPEC_RISE      = 0.37f;   // how quickly LEDs brighten. 1.0 = instantly
const float   SPEC_FALL      = 0.21f;   // how quickly LEDs dim. Bigger = faster, smaller = slower and calmer
const float   TREBLE_TOP_HZ  = 6600;    // the red zone stops here. Above this the Pico mostly hears "folded back" cymbal hiss
                                        // from above 7680Hz (too high for it to record properly), which barely follows the beat
const float   SPECTRUM_FLOOR = 20;      // stops background hiss lighting LEDs when it's quiet. With the stereo inputs silence
                                          // measures about 10 per frequency, quiet music 15-65, normal music 30-3000

// Bass flash: the colour shows how hard the bass hit
const CRGB    FLASH_SOFT       = CRGB(0, 0, 255);       // light hits: blue
const CRGB    FLASH_MEDIUM     = CRGB(40, 140, 255);    // medium hits: light blue
const CRGB    FLASH_HARD       = CRGB(255, 255, 255);   // hardest hits: white
const uint8_t FLASH_BRIGHTNESS = 255;   // brightness (0-255) of a flash at its peak
const uint8_t FLASH_ATTACK     = 47;    // brightness added per frame as a flash turns on. 255 = instant, 47 = ~75ms
const uint8_t FLASH_DECAY      = 40;    // how quickly a flash fades out. Bigger = faster (tuned so busy songs still flash, not glow)
const int     BASS_GAP         = 22;    // minimum frames between bass hits (22 = 0.37s), so flashes, sky glows and fire flares
                                        // land on the beat (works for songs up to about 160 BPM)
const float   BASS_TRIGGER     = 1.7f;  // how far the bass must jump above its recent average to flash. Bigger = less sensitive
const float   BASS_QUIET       = 300;   // stops background hiss from flashing. Bigger = ignores more hiss, but quiet music stops
                                        // triggering (1000 found nothing at low volume; silence measures about 75 in this range)
const float   BASS_HARDEST     = 3.0f;  // starting point for "the hardest hit" (white). The flash then adjusts it to each song's own hits,
                                        // so calm songs reach white on their biggest hits too
const float   FLASH_LEARN      = 0.2f;  // how quickly the flash learns each song's typical hit (settles in about 5 hits)
const float   BASS_BOTTOM_HZ   = 100;   // frequencies from here...
const float   BASS_TOP_HZ      = 500;   // ...up to here count as bass. Lower = deeper bass only (e.g. 150 = kick drums)

// Hi-hat sparkles (bass flash mode): hi-hats and cymbals light up random LEDs on top of the bass flash
// Tuned on 5 songs: 70-90% of hi-hat hits land on the eighth-note rhythm, and none fire during silence
const float   HAT_LOW_HZ   = 5000;                  // hi-hats and cymbals: 5000Hz up to TREBLE_TOP_HZ (6600Hz). Above that is
                                                    // mostly steady "folded back" cymbal wash, which hides the actual hits
const float   HAT_TRIGGER  = 1.25f;                 // how far the treble must jump above its recent average to sparkle. Bigger = less sensitive
const float   HAT_QUIET    = 300;                   // stops quiet hiss from sparkling. Smaller = more sensitive to soft hi-hats
const int     HAT_GAP      = 10;                    // minimum frames between hi-hat hits (10 = 0.17s, eighth notes up to 180 BPM)
const int     HAT_SPARKLES = 8;                     // LEDs lit per hi-hat hit. Set to NUM_LEDS to flash the whole strand instead
const CRGB    HAT_COLOUR   = CRGB(255, 255, 255);   // white
const uint8_t HAT_DECAY    = 30;                    // how quickly sparkles fade. Bigger = shorter and snappier (30 = visible for about 0.25s)

// FastLED hues, for the settings below: 0 red, 32 orange, 64 yellow, 96 green, 128 aqua, 160 blue, 192 purple, 224 pink

// Mood glow: a slow colour gradient drifting along the strand. Bass-heavy music pulls it warm, bright treble pulls it cool.
const float   MOOD_WARM_HZ  = 900;     // music centred this low (or lower) glows MOOD_WARM_HUE. Measured: calm ~850-1500Hz,
const float   MOOD_COOL_HZ  = 2700;    // balanced ~1000-2500Hz, hyper ~1000-2800Hz. Centred this high (or higher) glows MOOD_COOL_HUE
const uint8_t MOOD_WARM_HUE = 0;       // red
const uint8_t MOOD_COOL_HUE = 150;     // blue-cyan (in between it passes through orange, yellow, green and aqua)
const uint8_t MOOD_SPREAD   = 40;      // how much the colour changes along the strand. 0 = one colour
const float   MOOD_DRIFT    = 0.4f;    // how fast the gradient drifts along the strand
const float   MOOD_SMOOTH   = 0.04f;   // how slowly the colour follows the music. Smaller = slower (0.04 = about half a second)
const uint8_t MOOD_MIN      = 40;      // brightness (0-255) in quiet parts; louder parts glow up to full

// Beat ripples: each bass hit sends a pulse out from the middle to both ends; hi-hats send small fast sparks
const CRGB    RIPPLE_SOFT   = CRGB(120, 0, 255);     // light hits: purple
const CRGB    RIPPLE_MEDIUM = CRGB(255, 0, 150);     // medium hits: pink
const CRGB    RIPPLE_HARD   = CRGB(255, 255, 255);   // hardest hits: white
// Tuned on 25 seconds of real audio (a 116 BPM song): about 2 bass ripples a second, all spaced on the beat
const float   RIPPLE_TRIGGER   = 1.6f;    // how big a bass jump starts a ripple (the bass flash uses 1.7). Smaller = more ripples
const int     RIPPLE_GAP       = 22;      // minimum frames between bass ripples (22 = 0.37s). Stops extra ripples landing
                                          // between beats (works for songs up to about 160 BPM)
const float   RIPPLE_SPEED     = 0.35f;   // LEDs travelled per frame (0.35 = middle to end in about 1 second)
const float   RIPPLE_FADE      = 0.975f;  // how much brightness a ripple keeps each frame. Smaller = fades out sooner
const float   RIPPLE_WIDTH_MIN = 3;       // how many LEDs wide a light hit's ripple is...
const float   RIPPLE_WIDTH_MAX = 6;       // ...and the hardest hit's
const uint8_t RIPPLE_GLOW      = 20;      // faint purple glow behind the ripples (0-255). 0 = black between ripples
const CRGB    SPARK_COLOUR     = CRGB(255, 255, 255);   // hi-hat sparks: white
const float   SPARK_SPEED      = 1.0f;    // sparks travel faster than ripples
const float   SPARK_FADE       = 0.92f;
const float   SPARK_WIDTH      = 1.5f;
// Vocal ripples: the singing / melody range sends its own gentler teal ripples (measured: only ~40% of its hits land
// on the drum beat, so it really does follow the vocals rather than the drums)
const CRGB    VOCAL_COLOUR     = CRGB(0, 200, 170);   // teal
const float   VOCAL_LOW_HZ     = 450;     // the vocal range
const float   VOCAL_HIGH_HZ    = 2400;
const float   VOCAL_TRIGGER    = 1.5f;    // about 1.5-2 vocal ripples a second on measured music. Smaller = more
const float   VOCAL_QUIET      = 300;     // stops hiss from making vocal ripples. Smaller = more sensitive at low volume
const int     VOCAL_GAP        = 12;      // minimum frames between vocal ripples (12 = 0.2s)
const float   VOCAL_SPEED      = 0.25f;   // drifts out more slowly than the bass ripples
const float   VOCAL_WIDTH      = 4;

// Starfield: stars twinkle on and fade out over a dim blue sky. Louder music = more stars, hi-hats add a couple
// more, and bass hits make the whole sky glow.
const CRGB    STAR_COLOUR_1 = CRGB(255, 255, 255);   // each star is a random mix of white...
const CRGB    STAR_COLOUR_2 = CRGB(0, 200, 255);     // ...and cyan
const CRGB    STAR_SKY      = CRGB(40, 0, 70);       // the sky: deep purple
const uint8_t STAR_SKY_MIN  = 50;      // sky brightness (0-255 of STAR_SKY) between bass hits; a hit glows it up to full
const float   STARS_PER_SEC = 6;       // new stars per second at average loudness (more when louder, fewer when quieter)
const uint8_t STAR_RISE     = 60;      // how quickly a star twinkles on. 255 = instantly
const uint8_t STAR_FADE     = 10;      // how quickly it fades out. Bigger = shorter-lived stars (10 = about 1.5 seconds)
const float   SKY_GLOW_FADE = 0.9f;    // how much of a bass glow is left each frame. Closer to 1 = longer glow

// Fire: flames flicker in from both ends towards the middle; the bass makes them taller and kick drums make them flare
const bool    FIRE_ICE        = false;  // true = ice (deep blue, cyan, white) instead of fire (red, orange, yellow)
// Tuned on hyper, balanced and calm songs: the flames now rise and fall with the bass (about 0.15s behind it)
const uint8_t FIRE_COOLING    = 60;     // how quickly flames cool. Bigger = shorter flames
const uint8_t FIRE_FLICKER    = 40;     // random extra heat at the base each frame, so the flames flicker even when the bass is steady
const float   FIRE_FOLLOW     = 0.4f;   // how quickly the flames follow the bass. 1.0 = instantly
//------------------------------------------------------------------------------------------------------

//------------------------------ DO NOT CHANGE (until you understand it) -------------------------------
#define DATA_PIN    0    // GP0: main LED strand
#define DATA_PIN2   1    // GP1: second LED strand
#define LEFT_AUDIO_PIN  A0   // left channel audio
#define RIGHT_AUDIO_PIN A1   // right channel audio
#define KNOB_PIN        A2   // sensitivity (or brightness) potentiometer, see KNOB_CONTROLS
const uint8_t SWITCH_PINS[3] = {16, 17, 18};  // mode switches, read as a 3-bit binary number (4s, 2s, 1s)
#define PEAK_LED_FIRST 10  // on-board LEDs GP10..GP15 show the loudest frequency
#define PEAK_LED_LAST  15

CRGB leds[NUM_LEDS];
CRGB leds2[NUM_LEDS2];

// FFT settings. 256 samples at 15360Hz gives 128 frequency "buckets", each 60Hz wide, up to 7680Hz.
// One frame of 256 samples takes about 17ms to record. Core 1 records frames back to back, so the display
// refreshes 60 times a second and no sound is missed between frames.
const uint16_t SAMPLES = 256;              // MUST be a power of 2
constexpr float SAMPLING_FREQ = 15360;     // Hz
const unsigned long SAMPLE_PERIOD_US = (unsigned long)(1000000.0f / SAMPLING_FREQ + 0.5f);

// The FFT bucket that holds a given frequency, so the ranges below still work if SAMPLES or SAMPLING_FREQ change
constexpr int bucket(float hz) { return (int)(hz * SAMPLES / SAMPLING_FREQ + 0.5f); }

// The frequency range of each band: low = 60 - 600Hz, mid = 600 - 3600Hz, high = 3600 - 6600Hz
const float BAND_EDGES_HZ[4] = {60, LOW_TOP_HZ, MID_TOP_HZ, TREBLE_TOP_HZ};
const CRGB  BAND_COLOURS[3]  = {LOW_COLOUR, MID_COLOUR, HIGH_COLOUR};

// Both channels share ONE FFT: left goes in as the "real" part and right as the "imaginary" part, and they're
// separated again afterwards (see stereoFFT()). That's half the work of two FFTs, with exactly the same result.
// float is noticeably faster than double on the Pico, and accurate enough here.
float vReal[SAMPLES], vImag[SAMPLES];
ArduinoFFT<float> fft(vReal, vImag, SAMPLES, SAMPLING_FREQ);
float hammingWindow[SAMPLES];   // the window shape, worked out once in setup()

// Loudness of each frequency bucket for each channel, and the two mixed together (their average),
// which is used by the bars, bass flash and peak LEDs. Two extra entries for majorPeak().
float leftMag[SAMPLES / 2 + 2], rightMag[SAMPLES / 2 + 2], mix[SAMPLES / 2 + 2];
float fftCheckError = -1;   // result of the start-up self-test (should be close to 0)

// Loudness of each frequency range in the latest frame, worked out every frame in measureBands()
float low, mid, hi, bass, hat;
float bassRatio;   // how far the bass is above its recent average (1.0 = normal)
float hatRatio;    // the same for the hi-hat range
bool bassHit;      // true on the frame a new bass hit starts (at most one every BASS_GAP frames)
bool hatHit;       // true on the frame a new hi-hat hit starts (at most one every HAT_GAP frames)
int routine = 0;   // which display mode the switches select

// Which frequencies each spectrum LED shows. Worked out once in setup() from SPECTRUM_ZONES.
enum Channel : uint8_t { LEFT_CH, RIGHT_CH };
struct SpectrumLed {
  uint8_t first, last;   // the FFT buckets this LED shows
  Channel channel;       // left or right
  Band band;             // low, mid or high
  float scale;           // 1 / the number of buckets, to give the average loudness per bucket
};
SpectrumLed spectrumMap1[NUM_LEDS], spectrumMap2[NUM_LEDS2];

// Each strand's LEDs, sorted into the two sides of its middle and listed from the middle outwards
// (on GP0, LEDs 1-3 carry on round from LED 38). The spectrum and the newer effects all use this,
// so they agree on where the middle is and which side is left or right. Worked out once in setup().
const int MAX_SIDE = (NUM_LEDS > NUM_LEDS2) ? NUM_LEDS : NUM_LEDS2;
struct StrandLayout {
  int count[2];                  // LEDs on each side. Side 0 = the higher-numbered side, side 1 = the lower-numbered side
  uint8_t order[2][MAX_SIDE];    // each side's LEDs (counted from 0), from the middle outwards
};
StrandLayout layout1, layout2;
StrandLayout barLayout1;   // the same for the bars, around their own middle (OFFSET)

// Declared here because they use the types above (the Arduino IDE would otherwise declare them too early)
Band ledBand(bool useZones, int i, int numLeds, float centre);
void buildLayout(StrandLayout &layout, int numLeds, float centre, int wrapLeds);
Channel sideChannel(int side);
void buildSpectrumMap(SpectrumLed *map, const StrandLayout &layout, int numLeds, float centre, bool useZones);
void buildSegment(SpectrumLed *map, const uint8_t *leds, int count, Band band, Channel channel);
void drawRipples(CRGB *strip, const StrandLayout &layout);
void drawBars(CRGB *strip, const StrandLayout &layout, float lo, float mid, float hi);
// One star in the starfield: how bright it is, its colour (0 = STAR_COLOUR_1 ... 255 = STAR_COLOUR_2), and whether it's still twinkling on
struct Star { uint8_t level, tint; bool rising; };
void drawStarfield(CRGB *strip, Star *stars, int numLeds, int newStars, uint8_t sky);
void drawFire(CRGB *strip, const StrandLayout &layout, uint8_t heat[2][MAX_SIDE], bool flare, float bassLevel);
void drawSpectrum(CRGB *strip, const SpectrumLed *map, float *smooth, float *average, float *peak, int numLeds);

// Running averages act as an automatic gain control: the lights respond to how much the sound
// in each range changes compared to its recent average, not just to how loud it is.
// They are updated every frame, whichever mode is selected, so switching modes never starts from a stale average.
const float AVG_COUNT = 75;   // frames (75 = about 1.25 seconds). Bigger = slower to adapt to volume changes
float lowAvg = 0, midAvg = 0, hiAvg = 0, bassAvg = 0, hatAvg = 0;

// After silence the averages are tiny, so the first beats of a song would look enormous (a "flashbang": white flashes,
// full bars). So an average is never allowed below PEAK_SHARE of the recent peak. Measured on real music: the average
// stays above about 30% of the recent peak, so this only kicks in after silence or in very sparse intros.
const float PEAK_SHARE = 0.25f;
const float PEAK_RELAX = 0.995f;   // how quickly the remembered peak fades (0.995 = halves in about 2.3 seconds)
float lowPeak = 0, midPeak = 0, hiPeak = 0, bassPeak = 0, hatPeak = 0;

// Audio recorded by core 1. It fills one buffer while core 0 works on the other, then swaps.
volatile uint16_t audioBuffer[2][2][SAMPLES];   // [buffer][channel: 0 = left, 1 = right][sample]
volatile uint8_t readyBuffer = 0;               // the buffer core 1 finished most recently
volatile uint16_t knobReading = 512;            // latest knob reading, 0 - 1023
float sensitivity = 1.0f;                       // from the sensitivity knob: 1 = as tuned, bigger = reacts more
unsigned long nextSampleTime;                   // core 1's timer

#define DEBUG_PLOT   0  // set to 1 and open the Serial Plotter to see the three-colour band values
// TUNING BUILD: streams every frame's audio data over USB in binary (not readable in the Serial Monitor).
// Packet: 0xA5 0x5A, mode, brightness, frame number (2 bytes), wait us (2), work us (2),
//         then 128 left and 128 right bucket loudnesses, each 2 bytes, encoded as round(log2(1 + loudness) * 3800)
#define DEBUG_STREAM 1
#define DEBUG_TIMING 0  // set to 1 and open the Serial Monitor to see the mode, timing, levels and knob once a second
#define DEBUG_SPECTRUM 0  // 1 = print each GP0 spectrum LED's average and peak loudness once a second (spectrum mode)
                          // 2 = also stream every frame's raw LED loudness, for tuning

#if DEBUG_SPECTRUM
float dbgRawSum[NUM_LEDS], dbgRawMax[NUM_LEDS], dbgFrameRaw[NUM_LEDS];   // raw loudness (no gain) per LED
uint32_t dbgFrames = 0;
#endif

//------------------------------------------------------------------------------------------------------

void setup() {
  Serial.begin(115200);

  FastLED.addLeds<WS2812, DATA_PIN, GRB>(leds, NUM_LEDS);    // change GRB to RGB if red and green are swapped
  FastLED.addLeds<WS2812, DATA_PIN2, GRB>(leds2, NUM_LEDS2);
  FastLED.setMaxPowerInVoltsAndMilliamps(5, MAX_MA);
  FastLED.setBrightness(BRIGHTNESS);
  FastLED.clear(true);

  for (uint8_t pin : SWITCH_PINS) pinMode(pin, INPUT);

  // The Hamming window shape, worked out once (the same formula arduinoFFT uses)
  for (int i = 0; i < SAMPLES; i++) hammingWindow[i] = 0.54f - 0.46f * cosf(TWO_PI * i / (SAMPLES - 1));
  fftCheckError = fftSelfTest();


  // Work out which frequencies each spectrum LED shows
  buildLayout(layout1, NUM_LEDS,  NUM_LEDS / 2 + SPECTRUM_OFFSET, WRAP_LEDS);
  buildLayout(barLayout1, NUM_LEDS, NUM_LEDS / 2 + OFFSET, WRAP_LEDS);
  buildLayout(layout2, NUM_LEDS2, NUM_LEDS2 / 2 + OFFSET2,        0);
  buildSpectrumMap(spectrumMap1, layout1, NUM_LEDS,  NUM_LEDS / 2 + SPECTRUM_OFFSET, true);    // GP0: from SPECTRUM_ZONES
  buildSpectrumMap(spectrumMap2, layout2, NUM_LEDS2, NUM_LEDS2 / 2 + OFFSET2,        false);   // GP1: equal thirds

  for (int pin = 8; pin <= 15; pin++) {   // flash the on-board LEDs (GP8..GP15) as a start-up test
    pinMode(pin, OUTPUT);
    digitalWrite(pin, HIGH);
    delay(100);
    digitalWrite(pin, LOW);
  }

  Serial.println("The NeoPicoBlinkophone is ready to blink!");
}

void loop() {
#if DEBUG_TIMING
  uint32_t t0 = micros();
#endif

#if DEBUG_STREAM
  uint32_t s0 = micros();
#endif
  readAudio();   // wait for core 1 to finish recording the next frame
#if DEBUG_STREAM
  uint32_t s1 = micros();
#endif

#if DEBUG_TIMING
  uint32_t t1 = micros();
#endif

  stereoFFT();   // fills leftMag[], rightMag[] and mix[]

#if DEBUG_TIMING
  uint32_t t2 = micros();
#endif

  measureBands();
  displayPeak(fft.majorPeak(mix, SAMPLES, SAMPLING_FREQ));

  routine = 0;
  for (uint8_t pin : SWITCH_PINS) routine = (routine << 1) | digitalRead(pin);

  // The switch on GP18 adds 1, GP17 adds 2, GP16 adds 4
  // (on the NeoPicoBlinkophone the switch labelled 16 adds 1 and the one labelled 18 adds 4)
  switch (routine) {
    case 0:  FastLED.clear();       break;   // all switches off: LEDs off
    case 1:  bassDetect();          break;   // 16:          whole strand flashes on bass hits, hi-hat sparkles
    case 2:  displayThreeColour();  break;   // 17:          blue/white/red bars from the middle
    case 3:  intensitySpectrum();   break;   // 17+16:       stereo spectrum analyser
    case 4:  moodGlow();            break;   // 18:          slow colour gradient, warm for bass, cool for treble
    case 5:  beatRipples();         break;   // 18+16:       pulses out from the middle on bass hits, sparks on hi-hats
    case 6:  starfield();           break;   // 18+17:       twinkling stars over a blue sky that glows on bass hits
    default: fire();                break;   // 18+17+16:    flames in from both ends, taller with the bass
  }

  // Brightness comes from the knob or the fixed settings; MAX_MA still protects the power supply at any setting
  if (KNOB_CONTROLS == KNOB_BRIGHTNESS) FastLED.setBrightness(knobBrightness());
  else                                  FastLED.setBrightness(routine == 1 ? FLASH_MODE_BRIGHTNESS : BRIGHTNESS);

#if DEBUG_TIMING
  uint32_t t3 = micros();
#endif

#if DEBUG_STREAM
  streamFrame(s1 - s0, micros() - s1);
#endif

  FastLED.show();   // send the colours stored in leds[] and leds2[] out to the strands

#if DEBUG_TIMING
  printTiming(t1 - t0, t2 - t1, t3 - t2, micros() - t3);
#endif
}

#if DEBUG_STREAM
// Send this frame's left and right spectrum, the mode and the timing to the computer, for offline tuning
void streamFrame(uint32_t waitUs, uint32_t workUs) {
  static uint8_t packet[10 + 2 * 2 * 128];
  static uint16_t frameNo = 0;
  auto put16 = [](uint8_t *p, uint32_t v) { if (v > 65535) v = 65535; p[0] = v & 0xFF; p[1] = v >> 8; };
  auto encode = [](float v) -> uint32_t { return (uint32_t)(log2f(1.0f + v) * 3800.0f + 0.5f); };

  packet[0] = 0xA5; packet[1] = 0x5A;
  packet[2] = routine;
  packet[3] = FastLED.getBrightness();
  put16(packet + 4, frameNo++);
  put16(packet + 6, waitUs);
  put16(packet + 8, workUs);
  for (int k = 0; k < 128; k++) {
    put16(packet + 10 + 2 * k,       encode(leftMag[k]));
    put16(packet + 10 + 256 + 2 * k, encode(rightMag[k]));
  }
  Serial.write(packet, sizeof(packet));
}
#endif

//----------------------------- CORE 1: records audio non-stop -----------------------------
// setup1() and loop1() run on the Pico's second core, at the same time as setup() and loop() on the first.

void setup1() {
  nextSampleTime = micros();
}

// Fill one buffer with SAMPLES readings of each channel, evenly spaced in time, read the knob,
// then tell core 0 it's ready and swap buffers. The timer carries straight on into the next buffer,
// so there are no gaps in the recording.
void loop1() {
  static uint8_t buf = 0;
  for (uint16_t i = 0; i < SAMPLES; i++) {
    audioBuffer[buf][0][i] = analogRead(LEFT_AUDIO_PIN);
    audioBuffer[buf][1][i] = analogRead(RIGHT_AUDIO_PIN);
    while (micros() - nextSampleTime < SAMPLE_PERIOD_US) {}   // wait for the next sample time
    nextSampleTime += SAMPLE_PERIOD_US;
  }
  knobReading = analogRead(KNOB_PIN);
  readyBuffer = buf;
  rp2040.fifo.push_nb(buf);   // wake up core 0 (skipped if it's busy and hasn't collected the last one)
  buf ^= 1;                   // record into the other buffer next
}

//------------------------------------------------------------------------------------------

// Core 0: wait for core 1 to finish a buffer, then copy it in: left channel to vReal, right channel to vImag
void readAudio() {
  rp2040.fifo.pop();                                     // wait until a buffer is ready
  while (rp2040.fifo.available()) rp2040.fifo.pop();     // if we fell behind, skip straight to the newest

  uint8_t b = readyBuffer;
  for (uint16_t i = 0; i < SAMPLES; i++) {
    vReal[i] = audioBuffer[b][0][i];
    vImag[i] = audioBuffer[b][1][i];
  }
}

// Turn the left (vReal) and right (vImag) recordings into the loudness of each frequency bucket for each channel.
void stereoFFT() {
  // Remove the 1.65V offset from each channel so it doesn't show up as a huge 0Hz "bass" signal
  float leftMean = 0, rightMean = 0;
  for (int i = 0; i < SAMPLES; i++) { leftMean += vReal[i]; rightMean += vImag[i]; }
  leftMean /= SAMPLES; rightMean /= SAMPLES;

  // ...and weigh both with the Hamming window
  for (int i = 0; i < SAMPLES; i++) {
    vReal[i] = (vReal[i] - leftMean)  * hammingWindow[i];
    vImag[i] = (vImag[i] - rightMean) * hammingWindow[i];
  }

  // The library's FFT shuffles vReal into "bit-reversed" order before it starts, but not vImag (it expects vImag
  // to be all zeros). Shuffle vImag the same way here first, so both arrays line up.
  for (int i = 0, j = 0; i < SAMPLES - 1; i++) {
    if (i < j) { float t = vImag[i]; vImag[i] = vImag[j]; vImag[j] = t; }
    int k = SAMPLES >> 1;
    while (k <= j) { j -= k; k >>= 1; }
    j += k;
  }

  fft.compute(FFTDirection::Forward);

  // Separate the two channels. With X = the FFT result, bucket k of each channel is:
  //   left  = (X[k] + conjugate of X[N-k]) / 2
  //   right = (X[k] - conjugate of X[N-k]) / 2i
  for (int k = 0; k < SAMPLES / 2 + 2; k++) {
    int nk = (SAMPLES - k) & (SAMPLES - 1);
    float a = vReal[k], b = vImag[k], c = vReal[nk], d = vImag[nk];
    leftMag[k]  = 0.5f * sqrtf((a + c) * (a + c) + (b - d) * (b - d));
    rightMag[k] = 0.5f * sqrtf((b + d) * (b + d) + (a - c) * (a - c));
    mix[k] = (leftMag[k] + rightMag[k]) * 0.5f;
  }
}

// Start-up check that stereoFFT() gives the same answer as working out a few buckets the slow, obvious way.
// Returns the biggest difference as a fraction of the loudest bucket (anything under about 0.001 is a pass).
float fftSelfTest() {
  auto leftSignal  = [](int n) { return 2048 + 500 * sinf(TWO_PI * 20.3f * n / SAMPLES) + 300 * sinf(TWO_PI * 5 * n / SAMPLES); };
  auto rightSignal = [](int n) { return 2048 + 400 * sinf(TWO_PI * 47.7f * n / SAMPLES) + 200 * cosf(TWO_PI * 90 * n / SAMPLES); };
  for (int n = 0; n < SAMPLES; n++) { vReal[n] = leftSignal(n); vImag[n] = rightSignal(n); }
  stereoFFT();

  float lMean = 0, rMean = 0;
  for (int n = 0; n < SAMPLES; n++) { lMean += leftSignal(n); rMean += rightSignal(n); }
  lMean /= SAMPLES; rMean /= SAMPLES;

  const int testBuckets[] = {5, 20, 21, 48, 90, 3};
  float worst = 0, loudest = 1;
  for (int k : testBuckets) {
    float lr = 0, li = 0, rr = 0, ri = 0;
    for (int n = 0; n < SAMPLES; n++) {
      float c = cosf(TWO_PI * k * n / SAMPLES), s = sinf(TWO_PI * k * n / SAMPLES);
      float l = (leftSignal(n) - lMean) * hammingWindow[n], r = (rightSignal(n) - rMean) * hammingWindow[n];
      lr += l * c; li -= l * s; rr += r * c; ri -= r * s;
    }
    float lRef = sqrtf(lr * lr + li * li), rRef = sqrtf(rr * rr + ri * ri);
    loudest = fmaxf(loudest, fmaxf(lRef, rRef));
    worst = fmaxf(worst, fmaxf(fabsf(leftMag[k] - lRef), fabsf(rightMag[k] - rRef)));
  }
  return worst / loudest;
}

// The knob position, smoothed so it doesn't flicker: 0 = all the way down, 1 = all the way up
float knobPosition() {
  static float smoothed = -1;
  if (smoothed < 0) smoothed = knobReading;
  smoothed += (knobReading - smoothed) * 0.15f;
  float k = smoothed / 1023.0f;
  return KNOB_REVERSED ? 1.0f - k : k;
}

// Brightness from the knob
uint8_t knobBrightness() {
  return KNOB_MIN + (uint8_t)(knobPosition() * (KNOB_MAX - KNOB_MIN) + 0.5f);
}

// Sensitivity from the knob: SENS_LEAST all the way down, exactly 1 (the tuned settings) in the middle, SENS_MOST all the way up
float knobSensitivity() {
  if (KNOB_CONTROLS != KNOB_SENSITIVITY) return 1.0f;
  float k = knobPosition();
  return (k < 0.5f) ? SENS_LEAST + (1.0f - SENS_LEAST) * k * 2
                    : 1.0f + (SENS_MOST - 1.0f) * (k - 0.5f) * 2;
}

// Apply the sensitivity to a "compared with its recent average" level (1.0 = average): the jump above or
// below average is made bigger or smaller, so every trigger, bar and spectrum LED reacts more or less
float sensitise(float ratio) {
  return 1.0f + (ratio - 1.0f) * sensitivity;
}

// Add up the mixed FFT buckets from first to last (inclusive)
float sumBuckets(int first, int last) {
  float sum = 0;
  for (int i = first; i <= last; i++) sum += mix[i];
  return sum;
}

// Update a running average with a new value
void updateAvg(float &avg, float &peak, float value) {
  avg += (value - avg) / AVG_COUNT;
  peak = fmaxf(value, peak * PEAK_RELAX);    // remember the recent peak...
  avg = fmaxf(avg, PEAK_SHARE * peak);       // ...and don't let the average drop too far below it (see PEAK_SHARE)
}

// Work out the loudness of each frequency range and keep the running averages up to date
void measureBands() {
  low  = sumBuckets(bucket(60),         bucket(LOW_TOP_HZ) - 1);   //   60 - 600Hz
  mid  = sumBuckets(bucket(LOW_TOP_HZ), bucket(MID_TOP_HZ) - 1);   //  600 - 3600Hz
  hi   = sumBuckets(bucket(MID_TOP_HZ), bucket(TREBLE_TOP_HZ) - 1);   // 3600 - 6600Hz (above is mostly folded-back hiss)
  bass = sumBuckets(bucket(BASS_BOTTOM_HZ), bucket(BASS_TOP_HZ));  //  100 - 500Hz
  hat  = sumBuckets(bucket(HAT_LOW_HZ), bucket(TREBLE_TOP_HZ) - 1);   // 5000 - 6600Hz

  updateAvg(lowAvg,  lowPeak,  low);
  updateAvg(midAvg,  midPeak,  mid);
  updateAvg(hiAvg,   hiPeak,   hi);
  updateAvg(bassAvg, bassPeak, bass);
  updateAvg(hatAvg,  hatPeak,  hat);

  sensitivity = knobSensitivity();
  bassRatio = sensitise((bass + 100) / (bassAvg + BASS_QUIET));
  hatRatio  = sensitise((hat + 100)  / (hatAvg + HAT_QUIET));

  // New hits, worked out here once for every preset: true only on the frame a hit starts (not every frame it stays
  // loud), and never closer together than the minimum gap, so the effects stay in time with the music
  static bool wasBass = false, wasHat = false;
  static int sinceBass = 1000, sinceHat = 1000;
  sinceBass++;
  sinceHat++;
  bool isBass = bassRatio > BASS_TRIGGER, isHat = hatRatio > HAT_TRIGGER;
  bassHit = isBass && !wasBass && sinceBass >= BASS_GAP;
  hatHit  = isHat  && !wasHat  && sinceHat  >= HAT_GAP;
  if (bassHit) sinceBass = 0;
  if (hatHit)  sinceHat = 0;
  wasBass = isBass;
  wasHat = isHat;
}

// Light one on-board LED for the octave of the loudest frequency:
// GP15 <200Hz, GP14 <400Hz, GP13 <800Hz, GP12 <1600Hz, GP11 <3200Hz, GP10 above that
void displayPeak(float peakHz) {
  static int litPin = -1;

  int pin = PEAK_LED_LAST;
  for (float limit = 200; peakHz >= limit && pin > PEAK_LED_FIRST; limit *= 2) pin--;   // each octave doubles the frequency

  if (pin != litPin) {   // only touch the pins when the octave changes
    if (litPin >= 0) digitalWrite(litPin, LOW);
    digitalWrite(pin, HIGH);
    litPin = pin;
  }
}

#if DEBUG_TIMING
// Print the mode, the average time spent in each part of the frame, the levels and the knob, once a second
// (wait = time spent waiting for core 1 to finish recording: the spare time left in each frame)
void printTiming(uint32_t waitUs, uint32_t fftUs, uint32_t drawUs, uint32_t showUs) {
  static uint32_t frames = 0, sWait = 0, sFft = 0, sDraw = 0, sShow = 0, lastPrint = 0;
  static float maxRatio = 0, maxHat = 0;
  static uint32_t hatHits = 0;
  frames++; sWait += waitUs; sFft += fftUs; sDraw += drawUs; sShow += showUs;
  maxRatio = fmaxf(maxRatio, bassRatio);
  maxHat = fmaxf(maxHat, hatRatio);
  if (hatHit) hatHits++;
  if (millis() - lastPrint >= 1000) {
    Serial.printf("mode:%d fps:%lu wait:%luus fft:%luus draw:%luus show:%luus | low:%.0f mid:%.0f hi:%.0f | bassRatio:%.2f hatHits:%lu | knob:%u sensitivity:%.2f brightness:%u | fftcheck:%.5f\n",
                  routine, frames, sWait / frames, sFft / frames, sDraw / frames, sShow / frames,
                  low, mid, hi, maxRatio, hatHits, knobReading, sensitivity, FastLED.getBrightness(), fftCheckError);
#if DEBUG_SPECTRUM
    if (dbgFrames > 0) {
      Serial.print("LEDavg");
      for (int i = 0; i < NUM_LEDS; i++) Serial.printf(",%.1f", dbgRawSum[i] / dbgFrames);
      Serial.print("\nLEDmax");
      for (int i = 0; i < NUM_LEDS; i++) Serial.printf(",%.0f", dbgRawMax[i]);
      Serial.print("\nLEDmap");
      for (int i = 0; i < NUM_LEDS; i++) Serial.printf(",%c%d-%d", spectrumMap1[i].channel == LEFT_CH ? 'L' : 'R', spectrumMap1[i].first, spectrumMap1[i].last);
      Serial.println();
      for (int i = 0; i < NUM_LEDS; i++) dbgRawSum[i] = dbgRawMax[i] = 0;
      dbgFrames = 0;
    }
#endif
    frames = sWait = sFft = sDraw = sShow = hatHits = 0;
    maxRatio = maxHat = 0;
    lastPrint = millis();
  }
}
#endif

//------------------------------------------ DISPLAY ROUTINES ------------------------------------------

// Bars that grow out from the centre of the strand: blue for low, then white for mid, then red for high
void displayThreeColour() {
  // Compare each band with its own recent average (1.0 = about average for that band)
  float lowLevel = sensitise(low / fmaxf(lowAvg, BAR_QUIET));
  float midLevel = sensitise(mid / fmaxf(midAvg, BAR_QUIET));
  float hiLevel  = sensitise(hi  / fmaxf(hiAvg,  BAR_QUIET));

  // Smooth the levels so the bars glide instead of jumping. With BAR_RISE and BAR_FALL at 1.0 they jump instantly.
  static float lowS = 0, midS = 0, hiS = 0;
  lowS += (lowLevel - lowS) * (lowLevel > lowS ? BAR_RISE : BAR_FALL);
  midS += (midLevel - midS) * (midLevel > midS ? BAR_RISE : BAR_FALL);
  hiS  += (hiLevel  - hiS)  * (hiLevel  > hiS  ? BAR_RISE : BAR_FALL);

#if DEBUG_PLOT
  Serial.printf("low:%.2f,mid:%.2f,hi:%.2f\n", lowLevel, midLevel, hiLevel);
#endif

  drawBars(leds,  barLayout1, lowS, midS, hiS);
  drawBars(leds2, layout2,    lowS, midS, hiS);
}

// How many LEDs out from the centre one band's bar reaches. A level of 1.0 is "about average".
int barLength(float level, int sideLeds, float size) {
  return constrain((int)((level - BAR_GATE) * sideLeds * size), 0, sideLeds);
}

// Draw the bars out from the middle on both sides (on GP0, the higher side carries on round to LEDs 1-3)
void drawBars(CRGB *strip, const StrandLayout &layout, float lo, float mid, float hi) {
  for (int side = 0; side < 2; side++) {
    int n = layout.count[side];
    int lowEnd  = barLength(lo, n, BAR_LOW_SIZE);
    int midEnd  = lowEnd + barLength(mid, n, BAR_MID_SIZE);
    int highEnd = midEnd + barLength(hi, n, BAR_HIGH_SIZE);
    for (int k = 0; k < n; k++) {   // k = steps out from the middle
      CRGB c = (k < lowEnd) ? LOW_COLOUR : (k < midEnd) ? MID_COLOUR : (k < highEnd) ? HIGH_COLOUR : CRGB::Black;
      strip[layout.order[side][k]] = c;
    }
  }
}

// The colour of a bass flash: blue for a light hit, light blue for a medium hit, white for the hardest (strength 0 - 1)
CRGB flashColour(float strength) {
  if (strength < 0.5f) return blend(FLASH_SOFT,   FLASH_MEDIUM, (uint8_t)(strength * 2 * 255));
  return                      blend(FLASH_MEDIUM, FLASH_HARD,   (uint8_t)((strength - 0.5f) * 2 * 255));
}

// Flash both strands on a strong bass hit, then fade.
// The flash ramps up over a few frames (FLASH_ATTACK) instead of jumping straight to full brightness.
void bassDetect() {
  static uint8_t level = 0;         // brightness of the flash right now
  static bool rising = false;       // true while a flash is turning on
  static float hitStrength = 0;     // how hard the latest hit was: 0 = just triggered, 1 = the hardest
  static float typicalHit = (BASS_TRIGGER + BASS_HARDEST) / 2;   // this song's typical hit, learned as it plays

  // A new flash on each new bass hit (see measureBands(): never closer together than BASS_GAP, so they stay on the beat)
  if (bassHit) {
    // A typical hit for this song is light blue, harder than usual goes towards white, softer towards blue
    float hardest = fmaxf(BASS_TRIGGER + 0.2f, BASS_TRIGGER + 2 * (typicalHit - BASS_TRIGGER));
    hitStrength = constrain((bassRatio - BASS_TRIGGER) / (hardest - BASS_TRIGGER), 0.0f, 1.0f);
    typicalHit += (bassRatio - typicalHit) * FLASH_LEARN;
    rising = true;
  }

  if (rising) {                                         // attack: ramp up
    level = qadd8(level, FLASH_ATTACK);
    if (level >= FLASH_BRIGHTNESS) { level = FLASH_BRIGHTNESS; rising = false; }
  } else {                                              // decay: fade straight back out, so each beat is its own flash
    level = scale8(level, 255 - FLASH_DECAY);
  }

  CRGB colour = flashColour(hitStrength);
  colour.nscale8(level);
  fill_solid(leds,  NUM_LEDS,  colour);
  fill_solid(leds2, NUM_LEDS2, colour);

  // Hi-hats and cymbals: sparkle on top of the flash
  static uint8_t sparkle1[NUM_LEDS], sparkle2[NUM_LEDS2];   // brightness of each LED's sparkle
  addSparkles(leds,  sparkle1, NUM_LEDS,  hatHit);
  addSparkles(leds2, sparkle2, NUM_LEDS2, hatHit);
}

// Light HAT_SPARKLES random LEDs on a hi-hat hit, and fade the existing sparkles
void addSparkles(CRGB *strip, uint8_t *sparkle, int numLeds, bool hit) {
  if (hit) {
    if (HAT_SPARKLES >= numLeds) {
      for (int i = 0; i < numLeds; i++) sparkle[i] = 255;            // whole strand
    } else {
      for (int n = 0; n < HAT_SPARKLES; n++) sparkle[random16(numLeds)] = 255;
    }
  }

  for (int i = 0; i < numLeds; i++) {
    if (sparkle[i] == 0) continue;
    CRGB c = HAT_COLOUR;
    c.nscale8(sparkle[i]);
    strip[i] += c;                                    // add on top of the flash colour
    sparkle[i] = scale8(sparkle[i], 255 - HAT_DECAY);
  }
}

//----------------------------------------- STEREO SPECTRUM --------------------------------------------

// Which band (low, mid or high) an LED belongs to. GP0 uses the SPECTRUM_ZONES table;
// GP1 uses equal thirds on each side of its centre (blue in the middle, then white, then red).
Band ledBand(bool useZones, int i, int numLeds, float centre) {
  if (useZones) {
    for (const SpectrumZone &zone : SPECTRUM_ZONES) {
      if (i + 1 <= zone.lastLed) return zone.band;   // the table counts LEDs from 1
    }
    return SPECTRUM_ZONES[sizeof(SPECTRUM_ZONES) / sizeof(SPECTRUM_ZONES[0]) - 1].band;
  }
  float sideEnd = (i < centre) ? centre : (numLeds - 1 - centre);
  float position = (sideEnd > 0) ? fabsf(i - centre) / sideEnd : 0;   // 0 at the centre, 1 at the end
  return (position < 1.0f / 3) ? BAND_LOW : (position < 2.0f / 3) ? BAND_MID : BAND_HIGH;
}

// Sort a strand's LEDs into the two sides of its middle, each listed from the middle outwards. Runs once, in setup().
// wrapLeds: how many LEDs at the start of the strand carry on from the end of the higher-numbered side
// (for a strand that loops round, so LED 1 sits next to the last LED).
void buildLayout(StrandLayout &layout, int numLeds, float centre, int wrapLeds) {
  int n = 0;
  for (int i = 0; i < numLeds; i++) if (i > centre) layout.order[0][n++] = i;   // higher-numbered side...
  for (int i = 0; i < wrapLeds; i++) layout.order[0][n++] = i;                // ...carrying on round to LED 1
  layout.count[0] = n;

  n = 0;
  for (int i = numLeds - 1; i >= wrapLeds; i--) if (i <= centre) layout.order[1][n++] = i;   // lower-numbered side
  layout.count[1] = n;
}

// Which audio channel each side of the strand shows
Channel sideChannel(int side) {
  return ((side == 0) == HIGHER_LEDS_ARE_RIGHT) ? RIGHT_CH : LEFT_CH;
}

// Work out which channel and which frequencies each spectrum LED shows. Runs once, in setup().
void buildSpectrumMap(SpectrumLed *map, const StrandLayout &layout, int numLeds, float centre, bool useZones) {
  for (int side = 0; side < 2; side++) {
    const uint8_t *order = layout.order[side];
    int n = layout.count[side];

    // Split each run of LEDs on this side that share a band into that band's frequencies,
    // spread like a piano keyboard: lowest next to the centre, highest furthest out.
    int start = 0;
    while (start < n) {
      Band band = ledBand(useZones, order[start], numLeds, centre);
      int end = start;
      while (end + 1 < n && ledBand(useZones, order[end + 1], numLeds, centre) == band) end++;
      buildSegment(map, order + start, end - start + 1, band, sideChannel(side));
      start = end + 1;
    }
  }
}

// Share one band's frequencies between a run of LEDs (listed from the centre outwards)
void buildSegment(SpectrumLed *map, const uint8_t *leds, int count, Band band, Channel channel) {
    float loHz = BAND_EDGES_HZ[band], hiHz = BAND_EDGES_HZ[band + 1];
    int topBucket = bucket(hiHz) - 1;
    if (topBucket > SAMPLES / 2 - 1) topBucket = SAMPLES / 2 - 1;
    int prevLast = bucket(loHz) - 1;

    for (int k = 0; k < count; k++) {                                    // k = 0 is the LED nearest the centre
      int i = leds[k];
      map[i].channel = channel;
      map[i].band = band;
      int first = prevLast + 1;
      int last;
      if (first > topBucket) {
        first = last = topBucket;                                        // more LEDs than buckets: repeat the top one
      } else if (k == count - 1) {
        last = topBucket;                                                // the outermost LED takes the rest
      } else {
        last = bucket(loHz * powf(hiHz / loHz, (k + 1.0f) / count)) - 1;
        if (last < first) last = first;                                  // every LED gets at least one bucket
        if (last > topBucket) last = topBucket;
      }
      map[i].first = first;
      map[i].last  = last;
      map[i].scale = 1.0f / (last - first + 1);
      prevLast = last;
    }
}

// Stereo spectrum analyser: right channel on the higher-numbered LEDs, left channel on the lower-numbered ones,
// blue (bass) next to the middle, then white (mids), then red (treble) at the ends.
void intensitySpectrum() {
  static float smooth1[NUM_LEDS], smooth2[NUM_LEDS2];     // each LED's smoothed brightness, 0 - 1
  static float average1[NUM_LEDS], average2[NUM_LEDS2];   // each LED's recent average loudness
  static float peak1[NUM_LEDS], peak2[NUM_LEDS2];         // ...and its recent peak (see PEAK_SHARE)
  drawSpectrum(leds,  spectrumMap1, smooth1, average1, peak1, NUM_LEDS);
  drawSpectrum(leds2, spectrumMap2, smooth2, average2, peak2, NUM_LEDS2);
}

void drawSpectrum(CRGB *strip, const SpectrumLed *map, float *smooth, float *average, float *peak, int numLeds) {
  for (int i = 0; i < numLeds; i++) {
    const SpectrumLed &led = map[i];
    const float *magnitude = (led.channel == LEFT_CH) ? leftMag : rightMag;

    // Average loudness of this LED's frequencies
    float sum = 0;
    for (int b = led.first; b <= led.last; b++) sum += magnitude[b];
    float value = sum * led.scale;

#if DEBUG_SPECTRUM
    if (strip == leds) {
      dbgFrameRaw[i] = value;
      dbgRawSum[i] += value;
      if (value > dbgRawMax[i]) dbgRawMax[i] = value;
      if (i == 0) dbgFrames++;
    }
#endif

    // Compare with this LED's own recent average: below SPEC_GATE times it = off, SPEC_FULL times it = full brightness
    if (average[i] <= 0) average[i] = value;                     // first frame: start from the current level
    float loudness = sensitise(value / fmaxf(average[i], SPECTRUM_FLOOR));
    average[i] += (value - average[i]) / SPEC_AVG_FRAMES;
    peak[i] = fmaxf(value, peak[i] * PEAK_RELAX);
    average[i] = fmaxf(average[i], PEAK_SHARE * peak[i]);        // no flashbang after silence (see PEAK_SHARE)
    float level = (loudness - SPEC_GATE) / (SPEC_FULL - SPEC_GATE);
    level = constrain(level, 0.0f, 1.0f);
    smooth[i] += (level - smooth[i]) * (level > smooth[i] ? SPEC_RISE : SPEC_FALL);   // glow and fade, don't flicker

    CRGB colour = BAND_COLOURS[led.band];
    colour.nscale8((uint8_t)(smooth[i] * SPECTRUM_MAX));
    strip[i] = colour;
  }

#if DEBUG_SPECTRUM == 2
  if (strip == leds) {   // one line per frame: F,<LED 1>,<LED 2>,...
    char line[NUM_LEDS * 8 + 4];
    int p = snprintf(line, sizeof(line), "F");
    for (int i = 0; i < numLeds; i++) p += snprintf(line + p, sizeof(line) - p, ",%d", (int)dbgFrameRaw[i]);
    line[p++] = '\n';
    Serial.write(line, p);
  }
#endif
}

//-------------------------------------------- MOOD GLOW -----------------------------------------------

// A slow colour gradient drifting along the strand. The colour follows the music's "spectral centre"
// (the frequency most of its sound is centred on): bass-heavy music glows warm, bright treble-heavy music glows cool.
void moodGlow() {
  static float tone = 0.5f;         // 0 = warmest, 1 = coolest (smoothed)
  static float bright = MOOD_MIN;   // smoothed brightness
  static float phase = 0;           // how far the gradient has drifted along the strand

  // Spectral centre: the loudness-weighted average frequency (skipped when it's quiet, so hiss doesn't change the colour)
  if (low + mid + hi > 3 * BAR_QUIET) {
    float total = 0, weighted = 0;
    for (int k = 1; k < bucket(TREBLE_TOP_HZ); k++) { total += mix[k]; weighted += mix[k] * k; }
    float hz = weighted / total * SAMPLING_FREQ / SAMPLES;
    float t = logf(hz / MOOD_WARM_HZ) / logf(MOOD_COOL_HZ / MOOD_WARM_HZ);   // 0 at MOOD_WARM_HZ, 1 at MOOD_COOL_HZ
    tone += (constrain(t, 0.0f, 1.0f) - tone) * MOOD_SMOOTH;
  }

  // Brightness follows how loud it is compared with the recent average (1.0 = about average)
  float loud = (low + mid + hi) / fmaxf(lowAvg + midAvg + hiAvg, 3 * BAR_QUIET);
  float target = MOOD_MIN + (255 - MOOD_MIN) * constrain(loud / 2, 0.0f, 1.0f);
  bright += (target - bright) * 0.1f;

  phase += MOOD_DRIFT;
  if (phase >= 256) phase -= 256;
  uint8_t hue = MOOD_WARM_HUE + (int)((MOOD_COOL_HUE - MOOD_WARM_HUE) * tone);
  paintMood(leds,  NUM_LEDS,  hue, (uint8_t)bright, (uint8_t)phase);
  paintMood(leds2, NUM_LEDS2, hue, (uint8_t)bright, (uint8_t)phase);
}

// One full, smooth wave of colour along the strand (it joins up where a looped strand meets itself)
void paintMood(CRGB *strip, int numLeds, uint8_t hue, uint8_t bright, uint8_t phase) {
  for (int i = 0; i < numLeds; i++) {
    int wave = (int)sin8((uint8_t)(i * 256 / numLeds + phase)) - 128;   // -128 to 127
    strip[i] = CHSV((uint8_t)(hue + wave * MOOD_SPREAD / 128), 255, bright);
  }
}

//------------------------------------------- BEAT RIPPLES ---------------------------------------------

// A pulse of light travelling out from the middle. bright = 0 means the slot is free.
struct Ripple { float pos, speed, fade, width, bright; CRGB colour; };
const int MAX_RIPPLES = 12;
Ripple ripples[MAX_RIPPLES];

// Start a new ripple in a free slot (or replace the faintest one)
void addRipple(float speed, float fade, float width, CRGB colour) {
  int slot = 0;
  for (int r = 1; r < MAX_RIPPLES; r++) if (ripples[r].bright < ripples[slot].bright) slot = r;
  ripples[slot] = {0, speed, fade, width, 1.0f, colour};
}

// Each bass hit sends a ripple out from the middle (purple, then pink, then white for harder hits, and wider);
// each hi-hat sends a small, fast white spark
void beatRipples() {
  static bool wasBass = false, wasVocal = false;
  static int sinceBass = 1000, sinceVocal = 1000;   // frames since the last ripple of each kind
  static float vocalAvg = -1;                       // the vocal range's recent average loudness
  sinceBass++;
  sinceVocal++;

  // Bass: a ripple on each new jump, but never closer together than RIPPLE_GAP, so they stay on the beat
  bool isBass = bassRatio > RIPPLE_TRIGGER;
  if (isBass && !wasBass && sinceBass >= RIPPLE_GAP) {
    float strength = constrain((bassRatio - RIPPLE_TRIGGER) / (BASS_HARDEST - RIPPLE_TRIGGER), 0.0f, 1.0f);
    CRGB colour = (strength < 0.5f) ? blend(RIPPLE_SOFT,   RIPPLE_MEDIUM, (uint8_t)(strength * 2 * 255))
                                    : blend(RIPPLE_MEDIUM, RIPPLE_HARD,   (uint8_t)((strength - 0.5f) * 2 * 255));
    addRipple(RIPPLE_SPEED, RIPPLE_FADE, RIPPLE_WIDTH_MIN + (RIPPLE_WIDTH_MAX - RIPPLE_WIDTH_MIN) * strength, colour);
    sinceBass = 0;
  }

  // Vocals: the same idea on the singing range, with its own colour
  float vocal = sumBuckets(bucket(VOCAL_LOW_HZ), bucket(VOCAL_HIGH_HZ));
  if (vocalAvg < 0) vocalAvg = vocal;                // first frame: start from the current level
  static float vocalPeak = 0;
  updateAvg(vocalAvg, vocalPeak, vocal);
  bool isVocal = sensitise((vocal + 100) / (vocalAvg + VOCAL_QUIET)) > VOCAL_TRIGGER;
  if (isVocal && !wasVocal && sinceVocal >= VOCAL_GAP) {
    addRipple(VOCAL_SPEED, RIPPLE_FADE, VOCAL_WIDTH, VOCAL_COLOUR);
    sinceVocal = 0;
  }

  // Hi-hats: small fast sparks
  if (hatHit) addRipple(SPARK_SPEED, SPARK_FADE, SPARK_WIDTH, SPARK_COLOUR);
  wasBass = isBass;
  wasVocal = isVocal;

  drawRipples(leds,  layout1);
  drawRipples(leds2, layout2);

  for (Ripple &r : ripples) {   // move every ripple outwards and fade it
    if (r.bright <= 0) continue;
    r.pos += r.speed;
    r.bright *= r.fade;
    if (r.bright < 0.02f || r.pos > MAX_SIDE + r.width) r.bright = 0;
  }
}

// Draw every ripple on both sides of one strand, at its distance from the middle
void drawRipples(CRGB *strip, const StrandLayout &layout) {
  CRGB glow = RIPPLE_SOFT;
  glow.nscale8(RIPPLE_GLOW);   // the faint background
  for (int side = 0; side < 2; side++) {
    for (int k = 0; k < layout.count[side]; k++) {
      CRGB c = glow;
      for (const Ripple &r : ripples) {
        if (r.bright <= 0) continue;
        float d = fabsf(k - r.pos);
        if (d >= r.width) continue;
        CRGB add = r.colour;
        add.nscale8((uint8_t)(255 * r.bright * (1 - d / r.width)));   // brightest at the centre of the ripple
        c += add;                                                      // overlapping ripples add together
      }
      strip[layout.order[side][k]] = c;
    }
  }
}

//--------------------------------------------- STARFIELD ----------------------------------------------

// Stars twinkle on at random spots and slowly fade, over a dim blue sky. The louder the music, the more stars;
// hi-hats add a couple of extra stars, and bass hits make the whole sky glow brighter for a moment.
void starfield() {
  static Star stars1[NUM_LEDS], stars2[NUM_LEDS2];
  static float glow = 0;       // extra sky brightness from the last bass hit, 0 - 1
  static float starsDue = 0;   // new stars owed so far (they come in fractions at 60 frames a second)

  if (bassHit) {   // a new bass hit: harder hits glow brighter
    float strength = constrain((bassRatio - BASS_TRIGGER) / (BASS_HARDEST - BASS_TRIGGER), 0.0f, 1.0f);
    glow = fmaxf(glow, 0.4f + 0.6f * strength);
  }

  // How many new stars this frame: STARS_PER_SEC at average loudness, scaled by how loud it is now
  float loud = (low + mid + hi) / fmaxf(lowAvg + midAvg + hiAvg, 3 * BAR_QUIET);   // 1.0 = about average
  starsDue += STARS_PER_SEC / 60.0f * loud;
  int newStars = (int)starsDue;
  starsDue -= newStars;
  if (hatHit) newStars += 2;

  uint8_t sky = STAR_SKY_MIN + (uint8_t)((255 - STAR_SKY_MIN) * glow);
  drawStarfield(leds,  stars1, NUM_LEDS,  newStars, sky);
  drawStarfield(leds2, stars2, NUM_LEDS2, newStars, sky);
  glow *= SKY_GLOW_FADE;
}

void drawStarfield(CRGB *strip, Star *stars, int numLeds, int newStars, uint8_t sky) {
  // Light new stars at random spots (only where it's dark, so stars don't pile up on each other)
  for (int n = 0; n < newStars; n++) {
    Star &s = stars[random16(numLeds)];
    if (s.level < 64) { s.rising = true; s.tint = random8(); }
  }

  for (int i = 0; i < numLeds; i++) {
    Star &s = stars[i];
    if (s.rising) {                                       // twinkle on quickly...
      s.level = qadd8(s.level, STAR_RISE);
      if (s.level == 255) s.rising = false;
    } else {
      s.level = scale8(s.level, 255 - STAR_FADE);         // ...then fade out slowly
    }

    CRGB c = STAR_SKY;
    c.nscale8(sky);
    CRGB star = blend(STAR_COLOUR_1, STAR_COLOUR_2, s.tint);
    star.nscale8(s.level);
    strip[i] = c + star;
  }
}

//----------------------------------------------- FIRE -------------------------------------------------

// Colours for the ice version: black, deep blue, cyan, white
CRGBPalette16 icePalette(CRGB::Black, CRGB(0, 0, 160), CRGB(0, 160, 255), CRGB::White);

// Flames flicker in from both ends of the strand towards the middle. More bass = taller, busier flames,
// and each kick drum makes the base of the flames flare up.
void fire() {
  static uint8_t heat1[2][MAX_SIDE], heat2[2][MAX_SIDE];   // how hot each spot in each flame is
  static float bassLevel = 0;

  float target = constrain((bass / fmaxf(bassAvg, BAR_QUIET) - 0.5f) / 1.5f, 0.0f, 1.0f);   // 0 = quiet bass, 1 = twice its average
  bassLevel += (target - bassLevel) * FIRE_FOLLOW;
  bool flare = bassHit;   // each new bass hit (on the beat, see measureBands()) makes the flames flare

  drawFire(leds,  layout1, heat1, flare, bassLevel);
  drawFire(leds2, layout2, heat2, flare, bassLevel);
}

// Fire2012 by Mark Kriegsman (from the FastLED examples), run on each side with the base of the flame at the outer end
void drawFire(CRGB *strip, const StrandLayout &layout, uint8_t heat[2][MAX_SIDE], bool flare, float bassLevel) {
  int cooling = FIRE_COOLING * (1.0f - 0.5f * bassLevel);                // more bass = less cooling = taller flames
  uint8_t baseHeat = (uint8_t)(bassLevel * 255);                         // the bass sets how hot the base of the flames is

  for (int side = 0; side < 2; side++) {
    int n = layout.count[side];
    if (n == 0) continue;
    uint8_t *h = heat[side];

    // 1. Every spot cools down a little
    int maxCool = cooling * 10 / n + 2;
    if (maxCool > 255) maxCool = 255;
    for (int i = 0; i < n; i++) h[i] = qsub8(h[i], random8(maxCool));

    // 2. Heat drifts from the base towards the middle, spreading out as it goes
    for (int i = n - 1; i >= 2; i--) h[i] = (h[i - 1] + h[i - 2] + h[i - 2]) / 3;

    // 3. Heat the base to match the bass (plus a little random flicker); a kick drum makes the whole base flare
    for (int y = 0; y < 3 && y < n; y++) {
      uint8_t heatNow = qadd8(baseHeat, random8(FIRE_FLICKER));
      if (h[y] < heatNow) h[y] = heatNow;
    }
    if (flare) for (int y = 0; y < 3 && y < n; y++) h[y] = 255;

    // 4. Turn heat into colour. h[0] is the outer end, so the LED k steps from the middle shows h[n - 1 - k]
    for (int k = 0; k < n; k++) {
      uint8_t t = h[n - 1 - k];
      strip[layout.order[side][k]] = FIRE_ICE ? ColorFromPalette(icePalette, scale8(t, 240)) : HeatColor(t);
    }
  }
}
