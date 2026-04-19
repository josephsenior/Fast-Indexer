using System;
using System.Runtime.InteropServices;
using System.Drawing;
using System.Drawing.Imaging;

[StructLayout(LayoutKind.Sequential)]
public struct SIZE { public int cx; public int cy; }

[Flags]
public enum SIIGBF : uint
{
    SIIGBF_RESIZETOFIT = 0x00,
    SIIGBF_BIGGERSIZEOK = 0x01,
    SIIGBF_MEMORYONLY = 0x02,
    SIIGBF_ICONONLY = 0x04,
    SIIGBF_THUMBNAILONLY = 0x08,
    SIIGBF_INCACHEONLY = 0x10
}

[ComImport]
[Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b")]
[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IShellItemImageFactory
{
    void GetImage(SIZE size, SIIGBF flags, out IntPtr phbm);
}

public static class ThumbnailExtractor
{
    [DllImport("shell32.dll", CharSet=CharSet.Unicode, PreserveSig=false)]
    private static extern int SHCreateItemFromParsingName([MarshalAs(UnmanagedType.LPWStr)] string pszPath, IntPtr pbc, [In] ref Guid riid, out IntPtr ppv);

    [DllImport("gdi32.dll")]
    private static extern bool DeleteObject(IntPtr hObject);

    public static void CreateThumbnail(string path, int width, int height, string outPath)
    {
        Guid iid = new Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b");
        IntPtr pFactory;
        int hr = SHCreateItemFromParsingName(path, IntPtr.Zero, ref iid, out pFactory);
        if (hr != 0) Marshal.ThrowExceptionForHR(hr);
        IShellItemImageFactory factory = (IShellItemImageFactory)Marshal.GetObjectForIUnknown(pFactory);
        SIZE size;
        size.cx = width;
        size.cy = height;
        IntPtr hBmp;
        factory.GetImage(size, SIIGBF.SIIGBF_RESIZETOFIT, out hBmp);
        using (Image img = Image.FromHbitmap(hBmp))
        {
            img.Save(outPath, ImageFormat.Png);
        }
        DeleteObject(hBmp);
        Marshal.Release(pFactory);
    }
}
