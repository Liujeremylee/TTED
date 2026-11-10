using System;
using System.Collections.Generic;
using System.IO;
using System.Text;

// Offline replay of the NeoPicoBlinkophone presets, using frames streamed from the Pico.
// Logic and default settings mirror the sketch; change fields on P to try alternatives.
public class P {
  public float AvgCount = 75; public float PeakShare = 0, PeakRelax = 0.995f;
  // bass flash
  public float BassTrig = 1.6f, BassHard = 3.0f, HatTrig = 1.3f;
  public int BassLo = 2, BassHi = 8, HatLo = 83, HatHi = 127;
  public float BassOff1 = 100, BassOff2 = 1000, HatOff1 = 100, HatOff2 = 500;
  public int FlashGap = 0, FlashDecay = 22; public bool FlashAdaptive = false; public float FlashLearn = 0.1f;
  // bars
  public float BarAvg = 75; public float BarQuiet = 300, BarGate = 0.3f, BarSizeLo = 0.30f, BarSizeMid = 0.30f, BarSizeHi = 0.30f, BarRise = 0.46f, BarFall = 0.18f;
  // spectrum
  public float SpecPeakShare = 0; public int SpecFrom = 0; public float SpecGate = 1.0f, SpecFull = 2.0f, SpecFloor = 4, SpecRise = 0.37f, SpecFall = 0.21f; public int SpecAvg = 120;
  // mood
  public float MoodWarm = 1250, MoodCool = 2500, MoodSmooth = 0.02f, MoodMin = 40;
  // ripples
  public float RipTrig = 1.4f, RipSpeed = 0.35f, RipFade = 0.975f, RipWMin = 3, RipWMax = 6; public int RipGap = 22;
  public int VocLo = 8, VocHi = 40; public float VocTrig = 1.4f, VocSpeed = 0.25f, VocWidth = 4; public int VocGap = 12;
  public float SparkSpeed = 1.0f, SparkFade = 0.92f, SparkWidth = 1.5f;
  // starfield
  public float StarsPerSec = 6; public int StarRise = 60, StarFade = 10;
  // fire
  public int FireCooling = 70, FireSparking = 80, FireBoost = 140; public int FireMode = 0; public int FireFeed = 120; public float FireFollow = 0.3f;
}

public class Sim {
  public int N;
  public float[][] L, R, M;
  public byte[] Mode;
  public string Name;

  public static Sim Load(string path, string name) {
    byte[] b = File.ReadAllBytes(path);
    var l = new List<float[]>(); var r = new List<float[]>(); var mo = new List<byte>();
    int i = 0;
    while (i + 522 <= b.Length) {
      if (b[i] == 0xA5 && b[i + 1] == 0x5A) {
        float[] fl = new float[128], fr = new float[128];
        for (int k = 0; k < 128; k++) {
          fl[k] = (float)(Math.Pow(2, (b[i + 10 + 2 * k] + 256 * b[i + 11 + 2 * k]) / 3800.0) - 1);
          fr[k] = (float)(Math.Pow(2, (b[i + 266 + 2 * k] + 256 * b[i + 267 + 2 * k]) / 3800.0) - 1);
        }
        l.Add(fl); r.Add(fr); mo.Add(b[i + 2]); i += 522;
      } else i++;
    }
    Sim s = new Sim(); s.Name = name; s.N = l.Count; s.L = l.ToArray(); s.R = r.ToArray(); s.Mode = mo.ToArray();
    s.M = new float[s.N][];
    for (int t = 0; t < s.N; t++) { s.M[t] = new float[128]; for (int k = 0; k < 128; k++) s.M[t][k] = (s.L[t][k] + s.R[t][k]) * 0.5f; }
    return s;
  }

  static float Sum(float[] a, int lo, int hi) { float s = 0; for (int k = lo; k <= hi; k++) s += a[k]; return s; }
  static float Clamp(float v, float lo, float hi) { return v < lo ? lo : (v > hi ? hi : v); }

  // Running band levels and ratios, exactly as measureBands()
  public void Bands(P p, out float[] low, out float[] mid, out float[] hi, out float[] lowA, out float[] midA, out float[] hiA,
                    out float[] bassR, out float[] hatR, out float[] bass, out float[] bassA) {
    low = new float[N]; mid = new float[N]; hi = new float[N]; lowA = new float[N]; midA = new float[N]; hiA = new float[N];
    bassR = new float[N]; hatR = new float[N]; bass = new float[N]; bassA = new float[N];
    float la = 0, ma = 0, ha = 0, ba = 0, hta = 0, pl = 0, pm = 0, ph = 0, pb = 0, pht = 0;
    for (int t = 0; t < N; t++) {
      float[] m = M[t];
      low[t] = Sum(m, 1, 9); mid[t] = Sum(m, 10, 59); hi[t] = Sum(m, 60, 109);
      bass[t] = Sum(m, p.BassLo, p.BassHi); float hat = Sum(m, p.HatLo, p.HatHi);
      if (t == 0) { la = low[t]; ma = mid[t]; ha = hi[t]; ba = bass[t]; hta = hat; }
      la += (low[t] - la) / p.AvgCount; ma += (mid[t] - ma) / p.AvgCount; ha += (hi[t] - ha) / p.AvgCount;
      ba += (bass[t] - ba) / p.AvgCount; hta += (hat - hta) / p.AvgCount;
      pl = Math.Max(low[t], pl * p.PeakRelax); pm = Math.Max(mid[t], pm * p.PeakRelax); ph = Math.Max(hi[t], ph * p.PeakRelax); pb = Math.Max(bass[t], pb * p.PeakRelax); pht = Math.Max(hat, pht * p.PeakRelax);
      la = Math.Max(la, p.PeakShare * pl); ma = Math.Max(ma, p.PeakShare * pm); ha = Math.Max(ha, p.PeakShare * ph); ba = Math.Max(ba, p.PeakShare * pb); hta = Math.Max(hta, p.PeakShare * pht);
      lowA[t] = la; midA[t] = ma; hiA[t] = ha; bassA[t] = ba;
      bassR[t] = (bass[t] + p.BassOff1) / (ba + p.BassOff2);
      hatR[t] = (hat + p.HatOff1) / (hta + p.HatOff2);
    }
  }

  // Tempo from the bass onsets (autocorrelation, 60 - 200 BPM)
  public float Bpm(P p) {
    float[] o = new float[N]; float mean = 0;
    for (int t = 1; t < N; t++) { o[t] = Math.Max(0, Sum(M[t], p.BassLo, p.BassHi) - Sum(M[t - 1], p.BassLo, p.BassHi)); mean += o[t]; }
    mean /= N; for (int t = 0; t < N; t++) o[t] -= mean;
    double best = 0; int bestLag = 30;
    for (int lag = 18; lag <= 60; lag++) { double s = 0; for (int t = lag; t < N; t++) s += o[t] * o[t - lag]; if (s > best) { best = s; bestLag = lag; } }
    return 3600f / bestLag;
  }

  // Share of gaps between events that land on the beat grid (within 4 frames of a whole number of beats)
  static float OnBeat(List<int> times, float beatFrames) {
    int ok = 0; if (times.Count < 2) return 0;
    for (int i = 1; i < times.Count; i++) { float x = (times[i] - times[i - 1]) / beatFrames; float f = Math.Abs(x - (float)Math.Round(x)); if (f * beatFrames <= 4 && Math.Round(x) >= 1) ok++; }
    return 100f * ok / (times.Count - 1);
  }

  // Spectral centre percentiles (Hz), for calibrating the mood glow
  public string Centroids(P p) {
    var c = new List<float>();
    for (int t = 0; t < N; t++) { float tot = 0, w = 0; for (int k = 1; k < 110; k++) { tot += M[t][k]; w += M[t][k] * k; } if (Sum(M[t], 1, 109) > 900) c.Add(w / tot * 60); }
    c.Sort();
    return string.Format("{0}: spectral centre 5%={1:N0} 25%={2:N0} 50%={3:N0} 75%={4:N0} 95%={5:N0} Hz", Name, c[c.Count / 20], c[c.Count / 4], c[c.Count / 2], c[c.Count * 3 / 4], c[c.Count * 19 / 20]);
  }

  // Hi-hat detector check: rate, how many gaps land on the half-beat (eighth-note) grid, and rhythm score of the band
  public string Hats(P p, int lo, int hi, float off1, float off2, float trig, int gap, string label) {
    float bpm = Bpm(p), half = 1800f / bpm;
    float[] e = new float[N]; for (int t = 0; t < N; t++) e[t] = Sum(M[t], lo, hi);
    float avg = e[0]; bool was = false; int since = 1000; var times = new List<int>();
    for (int t = 0; t < N; t++) { since++; avg += (e[t] - avg) / p.AvgCount; bool h = (e[t] + off1) / (avg + off2) > trig; if (h && !was && since >= gap) { times.Add(t); since = 0; } was = h; }
    int ok = 0; for (int i = 1; i < times.Count; i++) { float x = (times[i] - times[i - 1]) / half; float fr = Math.Abs(x - (float)Math.Round(x)); if (fr * half <= 3 && Math.Round(x) >= 1) ok++; }
    float[] o = new float[N]; float mean = 0; for (int t = 1; t < N; t++) { o[t] = Math.Max(0, e[t] - e[t - 1]); mean += o[t]; } mean /= N; for (int t = 0; t < N; t++) o[t] -= mean;
    double z = 0; for (int t = 0; t < N; t++) z += o[t] * o[t]; double best = 0; int lagLo = (int)(half * 0.8f), lagHi = (int)(half * 1.25f) + 1;
    for (int lag = lagLo; lag <= lagHi; lag++) { double s = 0; for (int t = lag; t < N; t++) s += o[t] * o[t - lag]; best = Math.Max(best, s / z); }
    return string.Format("  {0,-9} {1,-42} {2,4:N1}/s | on the half-beat grid {3,3:N0}% | eighth-note rhythm score {4:N2}", Name, label, times.Count / (N / 60f), 100f * ok / Math.Max(1, times.Count - 1), best);
  }

  // Silence -> music start: what each preset does while silent, and in the first seconds of music
  public string Timeline(P p, int forceOnset) {
    float[] low, mid, hi, lowA, midA, hiA, bassR, hatR, bass, bassA;
    Bands(p, out low, out mid, out hi, out lowA, out midA, out hiA, out bassR, out hatR, out bass, out bassA);
    var sb = new StringBuilder();
    float[] tot = new float[N]; for (int t = 0; t < N; t++) tot[t] = low[t] + mid[t] + hi[t];
    var q = new List<float>(tot); q.Sort(); float quiet = q[N / 10];
    int onset = forceOnset; if (onset < 0) for (int t = 0; t < N - 30; t++) { bool ok = true; for (int u = 0; u < 30; u++) if (tot[t + u] < quiet * 20) { ok = false; break; } if (ok) { onset = t; break; } }
    sb.AppendFormat("{0}: silence level ~{1:N0}, music starts at {2:N1}s\n", Name, quiet, onset / 60f);
    // hits with the shared detector (bass gap 22, hat gap 10)
    bool[] bHit = new bool[N], hHit = new bool[N]; bool wb = false, wh = false; int sb2 = 1000, sh = 1000;
    for (int t = 0; t < N; t++) { sb2++; sh++; bool ib = bassR[t] > p.BassTrig, ih = hatR[t] > p.HatTrig; bHit[t] = ib && !wb && sb2 >= p.FlashGap; hHit[t] = ih && !wh && sh >= 10; if (bHit[t]) sb2 = 0; if (hHit[t]) sh = 0; wb = ib; wh = ih; }
    int silB = 0, silH = 0; for (int t = 0; t < (onset < 0 ? N : onset); t++) { if (bHit[t]) silB++; if (hHit[t]) silH++; }
    sb.AppendFormat("  during silence: {0} bass hits, {1} hi-hat hits (should be 0)\n", silB, silH);
    // bass flash (adaptive colours) + bars (gate/size from p, 18 LEDs per side) + spectrum-ish overall level, per 0.25s
    float level = 0, hs = 0, typical = (p.BassTrig + p.BassHard) / 2; bool rising = false;
    float[] sm = new float[3];
    sb.AppendLine("  time    sound   bassRatio  flash brightness/colour   bars lit/18   new bass hits  hi-hats");
    int from = Math.Max(0, onset - 60), to = Math.Min(N, onset + 300);
    float rowMaxR = 0, rowMaxL = 0, rowCol = -1, rowBars = 0; int rowB = 0, rowH = 0, rowN = 0; float rowSound = 0;
    for (int t = 0; t < N; t++) {
      if (bHit[t]) { float hard = Math.Max(p.BassTrig + 0.2f, p.BassTrig + 2 * (typical - p.BassTrig)); hs = Clamp((bassR[t] - p.BassTrig) / (hard - p.BassTrig), 0, 1); typical += (bassR[t] - typical) * p.FlashLearn; rising = true; }
      if (rising) { level = Math.Min(255, level + 47); if (level >= 255) { level = 255; rising = false; } } else level = (float)Math.Floor(level * (255 - p.FlashDecay) / 256f);
      float[] lv = { low[t] / Math.Max(lowA[t], p.BarQuiet), mid[t] / Math.Max(midA[t], p.BarQuiet), hi[t] / Math.Max(hiA[t], p.BarQuiet) };
      float[] sz = { p.BarSizeLo, p.BarSizeMid, p.BarSizeHi }; int bt = 0;
      for (int b = 0; b < 3; b++) { sm[b] += (lv[b] - sm[b]) * (lv[b] > sm[b] ? p.BarRise : p.BarFall); bt += (int)Clamp((float)Math.Floor((sm[b] - p.BarGate) * 18 * sz[b]), 0, 18); }
      if (t < from || t >= to) continue;
      rowMaxR = Math.Max(rowMaxR, bassR[t]); if (level > rowMaxL) { rowMaxL = level; rowCol = hs; } rowBars += Math.Min(bt, 18); if (bHit[t]) rowB++; if (hHit[t]) rowH++; rowSound += tot[t]; rowN++;
      if (rowN == 15) {
        string col = rowMaxL < 30 ? "-" : (rowCol < 0.33f ? "blue" : rowCol < 0.67f ? "light blue" : "WHITE");
        sb.AppendFormat("  {0,5:N2}s {1,7:N0}   {2,5:N2}      {3,3:N0} {4,-12}           {5,4:N1}          {6}              {7}\n", (t - onset) / 60f, rowSound / rowN, rowMaxR, rowMaxL, col, rowBars / rowN, rowB, rowH);
        rowMaxR = 0; rowMaxL = 0; rowCol = -1; rowBars = 0; rowB = 0; rowH = 0; rowN = 0; rowSound = 0;
      }
    }
    return sb.ToString();
  }

  public string Report(P p) {
    float[] low, mid, hi, lowA, midA, hiA, bassR, hatR, bass, bassA;
    Bands(p, out low, out mid, out hi, out lowA, out midA, out hiA, out bassR, out hatR, out bass, out bassA);
    float secs = N / 60f, bpm = Bpm(p), beatF = 3600f / bpm;
    var sb = new StringBuilder();
    sb.AppendFormat("=== {0}: {1:N0}s, tempo ~{2:N0} BPM ===\n", Name, secs, bpm);

    // ---- 16: bass flash ----
    { int flashes = 0, lit = 0, sparks = 0; int[] col = new int[3]; bool was = false, wasH = false, rising = false; float level = 0, hs = 0;
      var times = new List<int>(); int since = 1000; float avgHit = (p.BassTrig + p.BassHard) / 2;
      for (int t = 0; t < N; t++) {
        since++;
        bool hit = bassR[t] > p.BassTrig;
        bool newFlash = hit && !was && since >= p.FlashGap;
        float hard = p.FlashAdaptive ? Math.Max(p.BassTrig + 0.2f, p.BassTrig + 2 * (avgHit - p.BassTrig)) : p.BassHard;
        if (newFlash) { float s = Clamp((bassR[t] - p.BassTrig) / (hard - p.BassTrig), 0, 1); hs = s; flashes++; times.Add(t); col[s < 0.33f ? 0 : (s < 0.67f ? 1 : 2)]++; rising = true; since = 0;
                        if (p.FlashAdaptive) avgHit += (bassR[t] - avgHit) * p.FlashLearn; }
        if (rising) { level = Math.Min(255, level + 47); if (level >= 255) { level = 255; rising = false; } } else if (!hit || p.FlashGap > 0) level = (float)Math.Floor(level * (255 - p.FlashDecay) / 256f);
        if (level > 25) lit++;
        bool h = hatR[t] > p.HatTrig; if (h && !wasH) sparks++; wasH = h; was = hit;
      }
      sb.AppendFormat("16    bass flash : {0,4:N1} flashes/s ({1:N0}% on beat) | strand lit {2,3:N0}% | blue/lt-blue/white {3:N0}/{4:N0}/{5:N0}% | sparkles {6:N1}/s\n",
        flashes / secs, OnBeat(times, beatF), 100f * lit / N, 100f * col[0] / Math.Max(1, flashes), 100f * col[1] / Math.Max(1, flashes), 100f * col[2] / Math.Max(1, flashes), sparks / secs);
    }
    // ---- 17: bars ----
    { float[] sm = new float[3], len = new float[3]; int full = 0, empty = 0; float sizeL = p.BarSizeLo, sizeM = p.BarSizeMid, sizeH = p.BarSizeHi;
      float ba0 = low[0], ba1 = mid[0], ba2 = hi[0]; double sumTot = 0, sumTot2 = 0;
      for (int t = 0; t < N; t++) {
        ba0 += (low[t] - ba0) / p.BarAvg; ba1 += (mid[t] - ba1) / p.BarAvg; ba2 += (hi[t] - ba2) / p.BarAvg;
        float[] lv = { low[t] / Math.Max(ba0, p.BarQuiet), mid[t] / Math.Max(ba1, p.BarQuiet), hi[t] / Math.Max(ba2, p.BarQuiet) };
        float[] sz = { sizeL, sizeM, sizeH }; int tot = 0;
        for (int b = 0; b < 3; b++) { sm[b] += (lv[b] - sm[b]) * (lv[b] > sm[b] ? p.BarRise : p.BarFall); int l = (int)Clamp((float)Math.Floor((sm[b] - p.BarGate) * 18 * sz[b]), 0, 18); len[b] += l; tot += l; }
        if (tot >= 18) full++; if (tot == 0) empty++; int tt = Math.Min(tot, 18); sumTot += tt; sumTot2 += tt * tt;
      }
      double mt = sumTot / N; sb.AppendFormat("17    bars       : blue {0:N1} white {1:N1} red {2:N1} LEDs per side | lit {5:N1}/18 +/-{6:N1} | full {3:N0}% | empty {4:N0}%\n", len[0] / N, len[1] / N, len[2] / N, 100f * full / N, 100f * empty / N, mt, Math.Sqrt(Math.Max(0, sumTot2 / N - mt * mt)));
    }
    // ---- 17+16: spectrum (GP0 map) ----
    { string mapS = "R76-85,R86-96,R97-109,L99-109,L90-98,L81-89,L73-80,L66-72,L60-65,L46-59,L36-45,L28-35,L22-27,L17-21,L13-16,L10-12,L7-9,L5-6,L4-4,L3-3,L2-2,L1-1,R1-1,R2-2,R3-3,R4-4,R5-6,R7-9,R10-12,R13-16,R17-21,R22-27,R28-35,R36-45,R46-59,R60-67,R68-75";
      string[] parts = mapS.Split(','); int n = parts.Length; float[] avg = new float[n], sm = new float[n], pk = new float[n]; int[] on = new int[n]; float bright = 0; int specFrom = p.SpecFrom;
      for (int t = 0; t < N; t++) for (int i = 0; i < n; i++) {
        string[] ab = parts[i].Substring(1).Split('-'); int a = int.Parse(ab[0]), z = int.Parse(ab[1]);
        float[] src = parts[i][0] == 'L' ? L[t] : R[t]; float v = Sum(src, a, z) / (z - a + 1);
        if (t == 0) avg[i] = v;
        float loud = v / Math.Max(avg[i], p.SpecFloor); avg[i] += (v - avg[i]) / p.SpecAvg; pk[i] = Math.Max(v, pk[i] * p.PeakRelax); avg[i] = Math.Max(avg[i], p.SpecPeakShare * pk[i]);
        float lv = Clamp((loud - p.SpecGate) / (p.SpecFull - p.SpecGate), 0, 1);
        sm[i] += (lv - sm[i]) * (lv > sm[i] ? p.SpecRise : p.SpecFall);
        if (t >= specFrom && t < specFrom + 300) { if (sm[i] > 0.1f) on[i]++; bright += sm[i]; }
      }
      int mn = 100, mx = 0; float tot = 0; int span = Math.Min(300, N - specFrom); for (int i = 0; i < n; i++) { int pc = 100 * on[i] / span; mn = Math.Min(mn, pc); mx = Math.Max(mx, pc); tot += pc; }
      sb.AppendFormat("17+16 spectrum   : LEDs lit {0:N0}% on average (least {1}%, most {2}%) | avg brightness {3:N0}%\n", tot / n, mn, mx, 100f * bright / (span * n));
    }
    // ---- 18: mood glow ----
    { float tone = 0.5f, br = p.MoodMin; var tones = new List<float>(); float brSum = 0; int atMin = 0;
      for (int t = 0; t < N; t++) {
        if (low[t] + mid[t] + hi[t] > 3 * p.BarQuiet) { float tot = 0, w = 0; for (int k = 1; k < 110; k++) { tot += M[t][k]; w += M[t][k] * k; }
          float hz = w / tot * 60; float x = (float)(Math.Log(hz / p.MoodWarm) / Math.Log(p.MoodCool / p.MoodWarm)); tone += (Clamp(x, 0, 1) - tone) * p.MoodSmooth; }
        float loud = (low[t] + mid[t] + hi[t]) / Math.Max(lowA[t] + midA[t] + hiA[t], 3 * p.BarQuiet);
        float target = p.MoodMin + (255 - p.MoodMin) * Clamp(loud / 2, 0, 1); br += (target - br) * 0.1f;
        tones.Add(tone); brSum += br; if (br < p.MoodMin + 15) atMin++;
      }
      tones.Sort(); float t10 = tones[N / 10], t50 = tones[N / 2], t90 = tones[N * 9 / 10];
      sb.AppendFormat("18    mood glow  : colour (0=red..1=blue) 10%={0:N2} median={1:N2} 90%={2:N2} | avg brightness {3:N0}/255 | near-dim {4:N0}%\n", t10, t50, t90, brSum / N, 100f * atMin / N);
    }
    // ---- 18+16: ripples ----
    { var rp = new List<float[]>(); // pos, speed, fade, width, bright
      bool wasB = false, wasH = false, wasV = false; int sinceB = 1000, sinceV = 1000; float vAvg = -1; int nb = 0, nv = 0, ns = 0; var times = new List<int>(); int lit = 0; float br = 0;
      for (int t = 0; t < N; t++) {
        sinceB++; sinceV++;
        bool isB = bassR[t] > p.RipTrig, isH = hatR[t] > p.HatTrig;
        if (isB && !wasB && sinceB >= p.RipGap) { float s = Clamp((bassR[t] - p.RipTrig) / (p.BassHard - p.RipTrig), 0, 1); rp.Add(new float[] { 0, p.RipSpeed, p.RipFade, p.RipWMin + (p.RipWMax - p.RipWMin) * s, 1 }); sinceB = 0; nb++; times.Add(t); }
        float voc = Sum(M[t], p.VocLo, p.VocHi); if (vAvg < 0) vAvg = voc; vAvg += (voc - vAvg) / p.AvgCount;
        bool isV = (voc + 100) / (vAvg + 1000) > p.VocTrig;
        if (isV && !wasV && sinceV >= p.VocGap) { rp.Add(new float[] { 0, p.VocSpeed, p.RipFade, p.VocWidth, 1 }); sinceV = 0; nv++; }
        if (isH && !wasH) { rp.Add(new float[] { 0, p.SparkSpeed, p.SparkFade, p.SparkWidth, 1 }); ns++; }
        wasB = isB; wasH = isH; wasV = isV;
        while (rp.Count > 12) rp.RemoveAt(0);
        for (int k = 0; k < 18; k++) { float v = 0; foreach (float[] r in rp) { float d = Math.Abs(k - r[0]); if (d < r[3]) v += r[4] * (1 - d / r[3]); } v = Math.Min(1, v); if (v > 0.1f) lit++; br += v; }
        for (int i = rp.Count - 1; i >= 0; i--) { rp[i][0] += rp[i][1]; rp[i][4] *= rp[i][2]; if (rp[i][4] < 0.02f || rp[i][0] > 37 + rp[i][3]) rp.RemoveAt(i); }
      }
      sb.AppendFormat("18+16 ripples    : bass {0:N1}/s ({1:N0}% on beat) | vocal {2:N1}/s | sparks {3:N1}/s | side lit {4:N0}% | avg brightness {5:N0}%\n", nb / secs, OnBeat(times, beatF), nv / secs, ns / secs, 100f * lit / (N * 18), 100f * br / (N * 18));
    }
    // ---- 18+17: starfield ----
    { Random rnd = new Random(1); int[] lvl = new int[37]; bool[] up = new bool[37]; float due = 0; bool wasB = false, wasH = false; int glows = 0; long vis = 0;
      for (int t = 0; t < N; t++) {
        bool isB = bassR[t] > p.BassTrig, isH = hatR[t] > p.HatTrig; if (isB && !wasB) glows++;
        float loud = (low[t] + mid[t] + hi[t]) / Math.Max(lowA[t] + midA[t] + hiA[t], 3 * p.BarQuiet);
        due += p.StarsPerSec / 60f * loud; int ns = (int)due; due -= ns; if (isH && !wasH) ns += 2; wasB = isB; wasH = isH;
        for (int s = 0; s < ns; s++) { int i = rnd.Next(37); if (lvl[i] < 64) up[i] = true; }
        for (int i = 0; i < 37; i++) { if (up[i]) { lvl[i] = Math.Min(255, lvl[i] + p.StarRise); if (lvl[i] == 255) up[i] = false; } else lvl[i] = lvl[i] * (255 - p.StarFade) / 256; if (lvl[i] > 25) vis++; }
      }
      sb.AppendFormat("18+17 starfield  : {0:N1} stars visible on average (of 37) | sky glows {1:N1}/s\n", (float)vis / N, glows / secs);
    }
    // ---- 18+17+16: fire ----
    { Random rnd = new Random(2); int n = 18; int[] h = new int[n]; float bl = 0; bool wasB = false; int flares = 0; long hot = 0; float[] cov = new float[N], bls = new float[N];
      for (int t = 0; t < N; t++) {
        float target = Clamp((bass[t] / Math.Max(bassA[t], p.BarQuiet) - 0.5f) / 1.5f, 0, 1); bl += (target - bl) * p.FireFollow;
        bool isB = bassR[t] > p.BassTrig; bool flare = isB && !wasB; wasB = isB; if (flare) flares++;
        int cooling = (int)(p.FireCooling * (1 - 0.5f * bl)); int sparking = Math.Min(255, p.FireSparking + (int)(p.FireBoost * bl));
        int maxCool = Math.Min(255, cooling * 10 / n + 2);
        for (int i = 0; i < n; i++) h[i] = Math.Max(0, h[i] - rnd.Next(maxCool));
        for (int i = n - 1; i >= 2; i--) h[i] = (h[i - 1] + 2 * h[i - 2]) / 3;
        if (p.FireMode == 0) { if (rnd.Next(256) < sparking) { int y = rnd.Next(3); h[y] = Math.Min(255, h[y] + 160 + rnd.Next(96)); } } else if (p.FireMode == 1) { for (int y = 0; y < 3; y++) h[y] = Math.Min(255, h[y] + (int)(bl * p.FireFeed) + rnd.Next(p.FireSparking)); } else { for (int y = 0; y < 3; y++) h[y] = Math.Max(h[y], Math.Min(255, (int)(bl * p.FireFeed) + rnd.Next(p.FireSparking))); }
        if (flare) for (int y = 0; y < 3; y++) h[y] = 255;
        int frameHot = 0; for (int i = 0; i < n; i++) if (h[i] > 80) frameHot++;
        hot += frameHot; cov[t] = frameHot / (float)n; bls[t] = bl;
      }
      // how much the flame height moves (smoothed over ~0.25s, like the eye sees it) and how well that follows the bass
      float[] cs = new float[N]; float a = 0; for (int t = 0; t < N; t++) { a += (cov[t] - a) * 0.25f; cs[t] = a; }
      double mc = 0, mb = 0; for (int t = 0; t < N; t++) { mc += cs[t]; mb += bls[t]; } mc /= N; mb /= N;
      double vc = 0, vb = 0; for (int t = 0; t < N; t++) { vc += (cs[t] - mc) * (cs[t] - mc); vb += (bls[t] - mb) * (bls[t] - mb); } double cv = -1e30; int bestLag = 0; for (int lag = 0; lag <= 30; lag++) { double c = 0; for (int t = lag; t < N; t++) c += (cs[t] - mc) * (bls[t - lag] - mb); if (c > cv) { cv = c; bestLag = lag; } }
      sb.AppendFormat("18+17+16 fire    : flames cover {0:N0}% of each side on average, moving +/-{1:N0}% | follows the bass {2:N2} (1 = perfectly, after {4} frames) | flares {3:N1}/s\n",
        100f * hot / (N * (float)n), 100 * Math.Sqrt(vc / N), cv / Math.Sqrt(vc * vb), flares / secs, bestLag);
    }
    return sb.ToString();
  }
}










