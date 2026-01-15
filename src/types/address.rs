//! 地址结构体实现
//!
//! 提供 IPv4/IPv6 地址的存储和转换功能

use std::fmt;
use std::hash::{Hash, Hasher};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::str::FromStr;

/// IPv4 地址类型标识
pub const ADDR_TYPE_IPV4: u8 = 4;
/// IPv6 地址类型标识
pub const ADDR_TYPE_IPV6: u8 = 6;

/// 地址类型枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressType {
    /// IPv4 地址
    Ipv4,
    /// IPv6 地址
    Ipv6,
}

/// 地址结构体
///
/// 支持 IPv4 和 IPv6 地址的存储，内部使用标准库的 `SocketAddr`
#[derive(Debug, Clone)]
pub struct Address {
    /// 内部地址存储
    addr: SocketAddr,
}

impl Address {
    /// 从 IPv4 地址创建
    pub fn from_ipv4(ip: Ipv4Addr, port: u16) -> Self {
        Self {
            addr: SocketAddr::V4(SocketAddrV4::new(ip, port)),
        }
    }

    /// 从 IPv6 地址创建
    pub fn from_ipv6(ip: Ipv6Addr, port: u16) -> Self {
        Self {
            addr: SocketAddr::V6(SocketAddrV6::new(ip, port, 0, 0)),
        }
    }

    /// 从 `SocketAddr` 转换
    pub fn from_sockaddr(sock_addr: SocketAddr) -> Self {
        Self { addr: sock_addr }
    }

    /// 从原生 sockaddr 创建地址（类似C++版本的 from_sockaddr）
    ///
    /// 支持 IPv4 (sockaddr_in) 和 IPv6 (sockaddr_in6)
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn from_raw_sockaddr(
        sockaddr: *const libc::sockaddr,
        socklen: libc::socklen_t,
    ) -> Result<Self, AddressParseError> {
        unsafe {
            match (*sockaddr).sa_family as libc::c_int {
                libc::AF_INET => {
                    if socklen < std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t {
                        return Err(AddressParseError::InvalidFormat);
                    }
                    let addr_in = &*(sockaddr as *const libc::sockaddr_in);
                    let ip = Ipv4Addr::from(addr_in.sin_addr.s_addr.to_ne_bytes());
                    let port = u16::from_be(addr_in.sin_port);
                    Ok(Self::from_ipv4(ip, port))
                }
                libc::AF_INET6 => {
                    if socklen < std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t {
                        return Err(AddressParseError::InvalidFormat);
                    }
                    let addr_in6 = &*(sockaddr as *const libc::sockaddr_in6);
                    let ip = Ipv6Addr::from(addr_in6.sin6_addr.s6_addr);
                    let port = u16::from_be(addr_in6.sin6_port);
                    let scope_id = addr_in6.sin6_scope_id;
                    Ok(Self::from_ipv6_with_scope_id(ip, port, scope_id))
                }
                _ => Err(AddressParseError::InvalidFormat),
            }
        }
    }

    /// 从 IPv6 地址创建，带 scope_id
    fn from_ipv6_with_scope_id(ip: Ipv6Addr, port: u16, scope_id: u32) -> Self {
        Self {
            addr: SocketAddr::V6(SocketAddrV6::new(ip, port, 0, scope_id)),
        }
    }

    /// 转换为 `SocketAddr`
    pub fn to_sockaddr(&self) -> SocketAddr {
        self.addr
    }

    /// 获取地址类型
    ///
    /// 返回 `ADDR_TYPE_IPV4` 或 `ADDR_TYPE_IPV6`
    pub fn get_type(&self) -> u8 {
        match self.addr {
            SocketAddr::V4(_) => ADDR_TYPE_IPV4,
            SocketAddr::V6(_) => ADDR_TYPE_IPV6,
        }
    }

    /// 获取 sockaddr 长度
    ///
    /// IPv4 返回 16，IPv6 返回 28
    pub fn get_len(&self) -> usize {
        match self.addr {
            SocketAddr::V4(_) => std::mem::size_of::<libc::sockaddr_in>(),
            SocketAddr::V6(_) => std::mem::size_of::<libc::sockaddr_in6>(),
        }
    }

    /// 获取端口号
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// 获取 IP 地址
    pub fn ip(&self) -> SocketAddr {
        self.addr
    }

    /// 转换为 libc::sockaddr_storage
    ///
    /// 用于 libc 系统调用
    pub fn to_sockaddr_storage(&self) -> libc::sockaddr_storage {
        match self.addr {
            SocketAddr::V4(v4) => {
                let sockaddr = libc::sockaddr_in {
                    sin_family: libc::AF_INET as libc::sa_family_t,
                    sin_port: v4.port().to_be(),
                    sin_addr: libc::in_addr {
                        s_addr: u32::from_ne_bytes(v4.ip().octets()),
                    },
                    #[cfg(target_os = "linux")]
                    sin_zero: [0; 8],
                };
                let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
                unsafe {
                    std::ptr::copy(
                        &sockaddr as *const _ as *const u8,
                        &mut storage as *mut _ as *mut u8,
                        std::mem::size_of::<libc::sockaddr_in>(),
                    );
                }
                storage
            }
            SocketAddr::V6(v6) => {
                let sockaddr = libc::sockaddr_in6 {
                    sin6_family: libc::AF_INET6 as libc::sa_family_t,
                    sin6_port: v6.port().to_be(),
                    sin6_addr: libc::in6_addr {
                        s6_addr: v6.ip().octets(),
                    },
                    sin6_flowinfo: 0,
                    sin6_scope_id: v6.scope_id(),
                };
                let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
                unsafe {
                    std::ptr::copy(
                        &sockaddr as *const _ as *const u8,
                        &mut storage as *mut _ as *mut u8,
                        std::mem::size_of::<libc::sockaddr_in6>(),
                    );
                }
                storage
            }
        }
    }

    /// 转换为原始字节（用于哈希）
    pub fn to_bytes(&self) -> Vec<u8> {
        match self.addr {
            SocketAddr::V4(v4) => {
                let mut bytes = Vec::with_capacity(8);
                bytes.extend_from_slice(&v4.ip().octets());
                bytes.extend_from_slice(&v4.port().to_be_bytes());
                bytes
            }
            SocketAddr::V6(v6) => {
                let mut bytes = Vec::with_capacity(24);
                bytes.extend_from_slice(&v6.ip().octets());
                bytes.extend_from_slice(&v6.port().to_be_bytes());
                bytes.extend_from_slice(&v6.flowinfo().to_be_bytes());
                bytes.extend_from_slice(&v6.scope_id().to_be_bytes());
                bytes
            }
        }
    }

    /// 创建已连接的 UDP socket（类似 C++ 版本的 new_connected_udp_fd）
    ///
    /// 创建一个 UDP socket 并连接到当前地址
    /// 返回 raw fd，失败返回 -1
    #[cfg(unix)]
    pub fn new_connected_udp_fd(
        &self,
        buf_size: usize,
    ) -> Result<std::os::unix::io::RawFd, std::io::Error> {
        let fd = unsafe {
            libc::socket(
                self.get_type() as libc::c_int,
                libc::SOCK_DGRAM,
                libc::IPPROTO_UDP,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        // 设置非阻塞
        crate::set_nonblocking(fd)?;

        // 设置缓冲区大小
        crate::set_buf_size(fd, buf_size)?;

        // 连接到远程地址
        let sockaddr = self.to_sockaddr_storage();
        let len = self.get_len() as libc::socklen_t;
        unsafe {
            if libc::connect(fd, &sockaddr as *const _ as *const libc::sockaddr, len) != 0 {
                libc::close(fd);
                return Err(std::io::Error::last_os_error());
            }
        }

        Ok(fd)
    }

    /// 创建已连接的 UDP socket（Windows 版本）
    #[cfg(windows)]
    pub fn new_connected_udp_fd(
        &self,
        buf_size: usize,
    ) -> Result<std::os::windows::io::RawSocket, std::io::Error> {
        let fd = unsafe {
            libc::socket(
                self.get_type() as libc::c_int,
                libc::SOCK_DGRAM,
                libc::IPPROTO_UDP,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        // 设置非阻塞
        crate::set_nonblocking(fd)?;

        // 设置缓冲区大小
        crate::set_buf_size(fd, buf_size)?;

        // 连接到远程地址
        let sockaddr = self.to_sockaddr_storage();
        let len = self.get_len() as libc::socklen_t;
        unsafe {
            if libc::connect(fd, &sockaddr as *const _ as *const libc::sockaddr, len) != 0 {
                libc::closesocket(fd);
                return Err(std::io::Error::last_os_error());
            }
        }

        Ok(fd)
    }

    /// 转换为 IPv4 映射的 IPv6 地址 (::ffff:x.x.x.x)
    ///
    /// 用于 4to6 翻译模式
    pub fn to_ipv4_mapped_ipv6(&self) -> Option<Self> {
        match self.addr {
            SocketAddr::V4(v4) => {
                // 将 IPv4 地址转换为 IPv4 映射的 IPv6 地址
                let ipv6_addr = Ipv6Addr::new(
                    0x0000,
                    0x0000,
                    0x0000,
                    0x0000,
                    0x0000,
                    0xffff,
                    ((v4.ip().octets()[0] as u16) << 8) | (v4.ip().octets()[1] as u16),
                    ((v4.ip().octets()[2] as u16) << 8) | (v4.ip().octets()[3] as u16),
                );
                Some(Self::from_ipv6(ipv6_addr, v4.port()))
            }
            SocketAddr::V6(_) => None,
        }
    }

    /// 从 IPv4 映射的 IPv6 地址提取 IPv4 地址
    ///
    /// 用于 6to4 翻译模式
    pub fn from_ipv4_mapped_ipv6(&self) -> Option<Self> {
        match self.addr {
            SocketAddr::V6(v6) => {
                // 检查是否是 IPv4 映射的 IPv6 地址 (::ffff:x.x.x.x)
                // Ipv6Addr::new使用16位段，所以格式为：
                // segments = [0, 0, 0, 0, 0, 0xffff, ipv4_high, ipv4_low]
                // octets = [0,0, 0,0, 0,0, 0,0, 0,0, 0xff,0xff, x.x.x.x]
                //          0-1  2-3  4-5  6-7  8-9  10-11      12-15
                let octets = v6.ip().octets();
                if octets[0] == 0
                    && octets[1] == 0
                    && octets[2] == 0
                    && octets[3] == 0
                    && octets[4] == 0
                    && octets[5] == 0
                    && octets[6] == 0
                    && octets[7] == 0
                    && octets[8] == 0
                    && octets[9] == 0
                    && octets[10] == 0xff
                    && octets[11] == 0xff
                {
                    let ipv4_addr = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
                    Some(Self::from_ipv4(ipv4_addr, v6.port()))
                } else {
                    None
                }
            }
            SocketAddr::V4(_) => None,
        }
    }

    /// 获取底层 sockaddr_storage（用于系统调用）
    pub fn as_sockaddr_ptr(&self) -> (*const libc::sockaddr, libc::socklen_t) {
        let storage = self.to_sockaddr_storage();
        let len = self.get_len() as libc::socklen_t;
        let ptr = &storage as *const _ as *const libc::sockaddr;
        (ptr, len)
    }

    /// 获取可变底层 sockaddr_storage
    pub fn as_sockaddr_ptr_mut(&mut self) -> (*mut libc::sockaddr, libc::socklen_t) {
        let storage = self.to_sockaddr_storage();
        let len = self.get_len() as libc::socklen_t;
        let mut storage = storage;
        let ptr = &mut storage as *mut _ as *mut libc::sockaddr;
        (ptr, len)
    }
}

impl PartialEq for Address {
    fn eq(&self, other: &Self) -> bool {
        self.addr == other.addr
    }
}

impl Eq for Address {}

impl Hash for Address {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // 使用 SDBM 哈希函数（与 C++ 版本保持一致）
        let bytes = self.to_bytes();
        let hash = crate::sdbm(&bytes);
        hash.hash(state);
    }
}

impl FromStr for Address {
    type Err = AddressParseError;

    /// 从字符串解析地址
    ///
    /// 支持两种格式：
    /// - IPv4: `"1.2.3.4:443"`
    /// - IPv6: `"[2001:db8::1]:443"`
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // 处理 IPv6 方括号格式: [::1]:8080
        if s.starts_with('[') {
            let closing = match s.find(']') {
                Some(idx) => idx,
                None => return Err(AddressParseError::InvalidFormat),
            };
            let ip_part = &s[1..closing];
            let port_part = &s[closing + 1..];

            // 检查端口格式
            if !port_part.starts_with(':') {
                return Err(AddressParseError::InvalidFormat);
            }
            let port: u16 = port_part[1..]
                .parse()
                .map_err(|_| AddressParseError::InvalidPort)?;

            let ip: Ipv6Addr = ip_part.parse().map_err(|_| AddressParseError::InvalidIp)?;
            return Ok(Self::from_ipv6(ip, port));
        }

        // 处理 IPv4 格式: 1.2.3.4:443
        if let Some(last_colon) = s.rfind(':') {
            let ip_part = &s[..last_colon];
            let port_part = &s[last_colon + 1..];

            // 排除纯 IPv6 地址的情况
            if ip_part.contains(':') {
                return Err(AddressParseError::InvalidFormat);
            }

            let ip: Ipv4Addr = ip_part.parse().map_err(|_| AddressParseError::InvalidIp)?;
            let port: u16 = port_part
                .parse()
                .map_err(|_| AddressParseError::InvalidPort)?;
            return Ok(Self::from_ipv4(ip, port));
        }

        Err(AddressParseError::InvalidFormat)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.addr {
            SocketAddr::V4(v4) => write!(f, "{}:{}", v4.ip(), v4.port()),
            SocketAddr::V6(v6) => write!(f, "[{}]:{}", v6.ip(), v6.port()),
        }
    }
}

/// 地址解析错误
#[derive(Debug, PartialEq)]
pub enum AddressParseError {
    /// 格式错误
    InvalidFormat,
    /// 无效的 IP 地址
    InvalidIp,
    /// 无效的端口号
    InvalidPort,
}

impl fmt::Display for AddressParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddressParseError::InvalidFormat => write!(f, "invalid address format"),
            AddressParseError::InvalidIp => write!(f, "invalid IP address"),
            AddressParseError::InvalidPort => write!(f, "invalid port number"),
        }
    }
}

impl std::error::Error for AddressParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipv4_parse() {
        let addr = "127.0.0.1:8080".parse::<Address>().expect("Option unwrap failed");
        assert_eq!(addr.get_type(), ADDR_TYPE_IPV4);
        assert_eq!(addr.port(), 8080);
        assert_eq!(addr.to_string(), "127.0.0.1:8080");
    }

    #[test]
    fn test_ipv6_parse() {
        let addr = "[::1]:8080".parse::<Address>().expect("Option unwrap failed");
        assert_eq!(addr.get_type(), ADDR_TYPE_IPV6);
        assert_eq!(addr.port(), 8080);
        assert_eq!(addr.to_string(), "[::1]:8080");
    }

    #[test]
    fn test_ipv6_any() {
        let addr = "[::]:443".parse::<Address>().expect("Option unwrap failed");
        assert_eq!(addr.get_type(), ADDR_TYPE_IPV6);
    }

    #[test]
    fn test_sockaddr_conversion() {
        let original: SocketAddr = "192.168.1.1:3000".parse().expect("Address parsing failed");
        let addr = Address::from_sockaddr(original);
        let converted = addr.to_sockaddr();
        assert_eq!(original, converted);
    }

    #[test]
    fn test_invalid_format() {
        assert_eq!(
            "invalid".parse::<Address>(),
            Err(AddressParseError::InvalidFormat)
        );
        assert_eq!(
            "127.0.0.1".parse::<Address>(),
            Err(AddressParseError::InvalidFormat)
        );
    }

    #[test]
    fn test_invalid_port() {
        assert_eq!(
            "127.0.0.1:abc".parse::<Address>(),
            Err(AddressParseError::InvalidPort)
        );
        assert_eq!(
            "127.0.0.1:99999".parse::<Address>(),
            Err(AddressParseError::InvalidPort)
        );
    }

    #[test]
    fn test_hash() {
        let addr1 = "127.0.0.1:8080".parse::<Address>().expect("Option unwrap failed");
        let addr2 = "127.0.0.1:8080".parse::<Address>().expect("Option unwrap failed");
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        addr1.hash(&mut hasher);
        let hash1 = hasher.finish();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        addr2.hash(&mut hasher);
        let hash2 = hasher.finish();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_eq() {
        let addr1 = "127.0.0.1:8080".parse::<Address>().expect("Option unwrap failed");
        let addr2 = "127.0.0.1:8080".parse::<Address>().expect("Option unwrap failed");
        let addr3 = "127.0.0.1:9090".parse::<Address>().expect("Option unwrap failed");
        assert_eq!(addr1, addr2);
        assert_ne!(addr1, addr3);
    }

    #[test]
    fn test_address_len() {
        let ipv4 = "127.0.0.1:8080".parse::<Address>().expect("Option unwrap failed");
        let ipv6 = "[::1]:8080".parse::<Address>().expect("Option unwrap failed");
        assert_eq!(ipv4.get_len(), std::mem::size_of::<libc::sockaddr_in>());
        assert_eq!(ipv6.get_len(), std::mem::size_of::<libc::sockaddr_in6>());
    }

    #[test]
    fn test_to_ipv4_mapped_ipv6() {
        let ipv4: Address = "192.168.1.1:8080".parse().expect("Address parsing failed");
        let ipv6_mapped = ipv4.to_ipv4_mapped_ipv6();
        assert!(ipv6_mapped.is_some());
        let mapped = ipv6_mapped.expect("Option unwrap failed");
        assert_eq!(mapped.get_type(), ADDR_TYPE_IPV6);
        // Check the mapped address format ::ffff:192.168.1.1
        let addr_str = mapped.to_string();
        assert!(addr_str.contains("192.168.1.1"));
    }

    #[test]
    fn test_from_ipv4_mapped_ipv6() {
        // First convert an IPv4 to mapped IPv6, then convert back
        let ipv4: Address = "192.168.1.1:8080".parse().expect("Address parsing failed");
        let ipv6_mapped = ipv4.to_ipv4_mapped_ipv6();
        assert!(ipv6_mapped.is_some());
        let ipv6_mapped = ipv6_mapped.expect("Option unwrap failed");
        let ipv4_back = ipv6_mapped.from_ipv4_mapped_ipv6();
        assert!(ipv4_back.is_some());
        let extracted = ipv4_back.expect("Option unwrap failed");
        assert_eq!(extracted.get_type(), ADDR_TYPE_IPV4);
        assert_eq!(extracted.to_string(), "192.168.1.1:8080");
    }

    #[test]
    fn test_non_mapped_ipv6() {
        // Regular IPv6 should not be converted
        let ipv6: Address = "[2001:db8::1]:8080".parse().expect("Address parsing failed");
        let ipv4 = ipv6.from_ipv4_mapped_ipv6();
        assert!(ipv4.is_none());
    }

    #[test]
    fn test_localhost_addresses() {
        let ipv4_localhost: Address = "127.0.0.1:8080".parse().expect("Address parsing failed");
        let ipv6_localhost: Address = "[::1]:8080".parse().expect("Address parsing failed");

        assert_eq!(ipv4_localhost.get_type(), ADDR_TYPE_IPV4);
        assert_eq!(ipv6_localhost.get_type(), ADDR_TYPE_IPV6);
        assert_eq!(ipv4_localhost.port(), 8080);
        assert_eq!(ipv6_localhost.port(), 8080);
    }

    #[test]
    fn test_address_port() {
        let addr: Address = "192.168.1.1:3000".parse().expect("Address parsing failed");
        assert_eq!(addr.port(), 3000);
    }

    #[test]
    fn test_unspecified_addresses() {
        let ipv4_any: Address = "0.0.0.0:0".parse().expect("Address parsing failed");
        let ipv6_any: Address = "[::]:0".parse().expect("Address parsing failed");

        assert_eq!(ipv4_any.get_type(), ADDR_TYPE_IPV4);
        assert_eq!(ipv6_any.get_type(), ADDR_TYPE_IPV6);
    }
}
