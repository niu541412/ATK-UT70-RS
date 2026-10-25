$ErrorActionPreference = 'Stop'

$source = @'
using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Drawing.Text;
using System.IO;

public static class Ut70Icon
{
    static Bitmap Draw(int size)
    {
        var bitmap = new Bitmap(size, size, PixelFormat.Format32bppArgb);
        using (var graphics = Graphics.FromImage(bitmap))
        {
            graphics.Clear(Color.Transparent);
            graphics.SmoothingMode = SmoothingMode.AntiAlias;
            graphics.TextRenderingHint = TextRenderingHint.AntiAliasGridFit;
            graphics.InterpolationMode = InterpolationMode.HighQualityBicubic;
            graphics.ScaleTransform(size / 512f, size / 512f);

            using (var tile = new GraphicsPath())
            {
                tile.AddArc(18, 18, 212, 212, 180, 90);
                tile.AddArc(282, 18, 212, 212, 270, 90);
                tile.AddArc(282, 282, 212, 212, 0, 90);
                tile.AddArc(18, 282, 212, 212, 90, 90);
                tile.CloseFigure();
                using (var gradient = new LinearGradientBrush(
                    new PointF(18, 18), new PointF(494, 494),
                    Color.FromArgb(27, 112, 205), Color.FromArgb(7, 91, 111)))
                {
                    graphics.FillPath(gradient, tile);
                }
            }

            using (var smallFont = new Font("Segoe UI", 54, FontStyle.Bold, GraphicsUnit.Pixel))
            using (var font = new Font("Segoe UI", 148, FontStyle.Bold, GraphicsUnit.Pixel))
            using (var format = new StringFormat())
            {
                format.Alignment = StringAlignment.Center;
                format.LineAlignment = StringAlignment.Center;
                graphics.DrawString("ATK", smallFont, Brushes.White,
                    new RectangleF(54, 58, 404, 110), format);
                graphics.DrawString("UT70", font, Brushes.White,
                    new RectangleF(38, 151, 436, 190), format);
            }

            using (var pen = new Pen(Color.FromArgb(172, 255, 241), 17))
            {
                pen.StartCap = LineCap.Round;
                pen.EndCap = LineCap.Round;
                pen.LineJoin = LineJoin.Round;
                graphics.DrawLines(pen, new PointF[] {
                    new PointF(99, 386), new PointF(182, 386),
                    new PointF(215, 345), new PointF(246, 422),
                    new PointF(281, 372), new PointF(342, 372),
                    new PointF(361, 386), new PointF(414, 386)
                });
            }
        }
        return bitmap;
    }

    static byte[] Png(int size)
    {
        using (var bitmap = Draw(size))
        using (var stream = new MemoryStream())
        {
            bitmap.Save(stream, ImageFormat.Png);
            return stream.ToArray();
        }
    }

    public static void Generate(string outputDirectory)
    {
        File.WriteAllBytes(Path.Combine(outputDirectory, "atk-ut70-rs.png"), Png(512));
        var sizes = new int[] { 16, 32, 48, 256 };
        var images = new byte[sizes.Length][];
        for (int i = 0; i < sizes.Length; i++) images[i] = Png(sizes[i]);

        using (var stream = File.Create(Path.Combine(outputDirectory, "atk-ut70-rs.ico")))
        using (var writer = new BinaryWriter(stream))
        {
            writer.Write((ushort)0);
            writer.Write((ushort)1);
            writer.Write((ushort)sizes.Length);
            int offset = 6 + sizes.Length * 16;
            for (int i = 0; i < sizes.Length; i++)
            {
                writer.Write((byte)(sizes[i] == 256 ? 0 : sizes[i]));
                writer.Write((byte)(sizes[i] == 256 ? 0 : sizes[i]));
                writer.Write((byte)0);
                writer.Write((byte)0);
                writer.Write((ushort)1);
                writer.Write((ushort)32);
                writer.Write(images[i].Length);
                writer.Write(offset);
                offset += images[i].Length;
            }
            foreach (var image in images) writer.Write(image);
        }
    }
}
'@

Add-Type -TypeDefinition $source -ReferencedAssemblies System.Drawing
$assets = Join-Path (Split-Path $PSScriptRoot -Parent) 'assets'
[Ut70Icon]::Generate($assets)
